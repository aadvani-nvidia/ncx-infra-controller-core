// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package handler

import (
	"context"
	"encoding/json"
	"net/http"
	"testing"

	"github.com/google/uuid"
	"github.com/labstack/echo/v4"
	"github.com/stretchr/testify/assert"
	"github.com/stretchr/testify/mock"
	"github.com/stretchr/testify/require"
	tmocks "go.temporal.io/sdk/mocks"
	tp "go.temporal.io/sdk/temporal"
	"google.golang.org/protobuf/encoding/protojson"

	"github.com/NVIDIA/infra-controller/rest-api/api/pkg/api/handler/util/common"
	"github.com/NVIDIA/infra-controller/rest-api/api/pkg/api/model"
	authz "github.com/NVIDIA/infra-controller/rest-api/auth/pkg/authorization"
	"github.com/NVIDIA/infra-controller/rest-api/common/pkg/grpcproxy"
	cutil "github.com/NVIDIA/infra-controller/rest-api/common/pkg/util"
	cdbm "github.com/NVIDIA/infra-controller/rest-api/db/pkg/db/model"
	corev1 "github.com/NVIDIA/infra-controller/rest-api/proto/core/gen/v1"
	swe "github.com/NVIDIA/infra-controller/rest-api/site-workflow/pkg/error"
)

func TestUpdateMachineSpxMultiplaneMaintenanceHandler_Handle(t *testing.T) {
	tests := []struct {
		name        string
		rail, plane string
		body        map[string]any
		setup       func(*testing.T, *common.TestSetupProviderMachineHandlerFixture)
		wantStatus  int
		wantProxy   bool
	}{
		{name: "provider disables maintenance on all rails with attached instance", rail: "all", plane: "6", body: map[string]any{"maintenanceMode": false}, setup: func(t *testing.T, f *common.TestSetupProviderMachineHandlerFixture) {
			_, err := cdbm.NewMachineDAO(f.DBSession).Update(context.Background(), nil, cdbm.MachineUpdateInput{MachineID: f.MachineID, IsAssigned: cutil.GetPtr(true)})
			require.NoError(t, err)
		}, wantStatus: http.StatusOK, wantProxy: true},
		{name: "uses path selectors and clears reference", rail: "0", plane: "0", body: map[string]any{"maintenanceMode": true, "reference": "", "rail": "3", "plane": 7, "machineId": "different-machine"}, wantStatus: http.StatusOK, wantProxy: true},
		{name: "requires explicit maintenance mode", rail: "0", plane: "0", body: map[string]any{}, wantStatus: http.StatusBadRequest},
		{name: "validates rail selector", rail: "4", plane: "0", body: map[string]any{"maintenanceMode": true}, wantStatus: http.StatusBadRequest},
		{name: "rejects nonnumeric plane", rail: "0", plane: "six", body: map[string]any{"maintenanceMode": true}, wantStatus: http.StatusBadRequest},
		{name: "requires provider admin", rail: "0", plane: "0", body: map[string]any{"maintenanceMode": true}, setup: func(_ *testing.T, f *common.TestSetupProviderMachineHandlerFixture) {
			f.User = &cdbm.User{OrgData: cdbm.OrgData{f.Org: cdbm.Org{Name: f.Org, Roles: []string{authz.ProviderViewerRole}}}}
		}, wantStatus: http.StatusForbidden},
		{name: "hides another provider's machine", rail: "0", plane: "0", body: map[string]any{"maintenanceMode": true}, setup: func(t *testing.T, f *common.TestSetupProviderMachineHandlerFixture) {
			f.Org = "other-org"
			user := common.TestBuildUser(t, f.DBSession, "other-user", f.Org, []string{authz.ProviderAdminRole})
			common.TestBuildInfrastructureProvider(t, f.DBSession, "Other Provider", f.Org, user)
			f.User = user
		}, wantStatus: http.StatusNotFound},
		{name: "rejects site that is not registered", rail: "0", plane: "0", body: map[string]any{"maintenanceMode": true}, setup: func(t *testing.T, f *common.TestSetupProviderMachineHandlerFixture) {
			_, err := cdbm.NewSiteDAO(f.DBSession).Update(context.Background(), nil, cdbm.SiteUpdateInput{SiteID: uuid.MustParse(f.SiteID), Status: cutil.GetPtr(cdbm.SiteStatusPending)})
			require.NoError(t, err)
		}, wantStatus: http.StatusPreconditionFailed},
		{name: "preserves Core declared-interface validation failure", rail: "all", plane: "6", body: map[string]any{"maintenanceMode": true}, setup: func(t *testing.T, f *common.TestSetupProviderMachineHandlerFixture) {
			wrun := &tmocks.WorkflowRun{}
			wrun.On("Get", mock.Anything, mock.Anything).Return(tp.NewNonRetryableApplicationError("eth_r3_p6 is not declared", swe.ErrTypeNICoInvalidArgument, nil))
			client := &tmocks.Client{}
			client.On("ExecuteWorkflow", mock.Anything, mock.Anything, grpcproxy.Core.WorkflowName, mock.MatchedBy(func(req grpcproxy.Request) bool {
				*f.ProxiedReq = req
				return true
			})).Return(wrun, nil)
			f.SiteClientPool.IDClientMap[f.SiteID] = client
			t.Cleanup(func() { client.AssertExpectations(t); wrun.AssertExpectations(t) })
		}, wantStatus: http.StatusBadRequest, wantProxy: true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			coreResponse := &corev1.MachineSpxMultiplane{}
			fixture := common.NewTestSetupProviderMachineHandlerFixture(t, coreResponse)
			coreResponse.MachineId = &corev1.MachineId{Id: fixture.MachineID}
			coreResponse.Interfaces = []*corev1.MachineSpxMultiplaneInterface{{LogicalIfname: "eth_r2_p6", Rail: 2, Plane: 6, MaintenanceMode: false, Revision: "V2-T1791792000000000"}}
			if tt.setup != nil {
				tt.setup(t, &fixture)
			}
			handler := NewUpdateMachineSpxMultiplaneMaintenanceHandler(fixture.DBSession, fixture.SiteClientPool)
			rec := fixture.Request(t, func(c echo.Context) error {
				c.SetParamNames("orgName", "id", "rail", "plane")
				c.SetParamValues(fixture.Org, fixture.MachineID, tt.rail, tt.plane)
				return handler.Handle(c)
			}, http.MethodPatch, "/", tt.body, "")
			assert.Equal(t, tt.wantStatus, rec.Code, rec.Body.String())
			if !tt.wantProxy {
				assert.Empty(t, fixture.ProxiedReq.FullMethod)
				return
			}
			require.Equal(t, updateMachineSpxMultiplaneMaintenanceMethod, fixture.ProxiedReq.FullMethod)
			assert.Empty(t, fixture.ProxiedReq.EncryptedSecrets)
			var request corev1.MachineSpxMultiplaneMaintenanceUpdateRequest
			require.NoError(t, protojson.Unmarshal(fixture.ProxiedReq.RequestJSON, &request))
			assert.Equal(t, fixture.MachineID, request.GetMachineId().GetId())
			require.NotNil(t, request.MaintenanceMode)
			assert.Equal(t, tt.body["maintenanceMode"], request.GetMaintenanceMode())
			if tt.rail == "all" {
				require.NotNil(t, request.GetAllRails())
				assert.Equal(t, uint32(6), request.GetPlane())
			} else {
				require.IsType(t, &corev1.MachineSpxMultiplaneMaintenanceUpdateRequest_Rail{}, request.RailSelector)
				assert.Zero(t, request.GetRail())
				assert.Zero(t, request.GetPlane())
			}
			if reference, ok := tt.body["reference"]; ok {
				require.NotNil(t, request.Reference)
				assert.Equal(t, reference, request.GetReference())
			} else {
				assert.Nil(t, request.Reference)
			}
			if rec.Code == http.StatusOK {
				var response model.APIMachineSpxMultiplane
				require.NoError(t, json.Unmarshal(rec.Body.Bytes(), &response))
				assert.Equal(t, fixture.MachineID, response.MachineID)
				require.Len(t, response.Interfaces, 1)
				assert.Equal(t, "V2-T1791792000000000", response.Interfaces[0].Revision)
				assert.False(t, response.Interfaces[0].MaintenanceMode)
				assert.Equal(t, model.MachineSpxMultiplaneStatusPending, response.Interfaces[0].Status)
				assert.Nil(t, response.Interfaces[0].AppliedMaintenanceMode)
			}
		})
	}
}

func TestGetMachineSpxMultiplaneHandler_Handle(t *testing.T) {
	tests := []struct {
		name                             string
		missing, mismatch, missingOnSite bool
		wantStatus                       int
	}{
		{name: "provider viewer reads pending state", wantStatus: http.StatusOK},
		{name: "missing machine", missing: true, wantStatus: http.StatusNotFound},
		{name: "missing on site", missingOnSite: true, wantStatus: http.StatusPreconditionFailed},
		{name: "rejects mismatched site response", mismatch: true, wantStatus: http.StatusInternalServerError},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			coreResponse := &corev1.MachineSpxMultiplane{}
			fixture := common.NewTestSetupProviderMachineHandlerFixture(t, coreResponse)
			coreResponse.MachineId = &corev1.MachineId{Id: fixture.MachineID}
			if tt.mismatch {
				coreResponse.MachineId.Id = "another-machine"
			}
			fixture.User = &cdbm.User{OrgData: cdbm.OrgData{fixture.Org: cdbm.Org{Name: fixture.Org, Roles: []string{authz.ProviderViewerRole}}}}
			if tt.missing {
				fixture.MachineID = "missing-machine"
			}
			if tt.missingOnSite {
				_, err := cdbm.NewMachineDAO(fixture.DBSession).Update(context.Background(), nil, cdbm.MachineUpdateInput{MachineID: fixture.MachineID, IsMissingOnSite: cutil.GetPtr(true)})
				require.NoError(t, err)
			}
			handler := NewGetMachineSpxMultiplaneHandler(fixture.DBSession, fixture.SiteClientPool)
			rec := fixture.Request(t, handler.Handle, http.MethodGet, "/", nil, "")
			require.Equal(t, tt.wantStatus, rec.Code, rec.Body.String())
			if tt.missing || tt.missingOnSite {
				assert.Empty(t, fixture.ProxiedReq.FullMethod)
				return
			}
			assert.Equal(t, getMachineSpxMultiplaneMethod, fixture.ProxiedReq.FullMethod)
			var request corev1.MachineSpxMultiplaneRequest
			require.NoError(t, protojson.Unmarshal(fixture.ProxiedReq.RequestJSON, &request))
			assert.Equal(t, fixture.MachineID, request.GetMachineId().GetId())
			if rec.Code == http.StatusOK {
				var response model.APIMachineSpxMultiplane
				require.NoError(t, json.Unmarshal(rec.Body.Bytes(), &response))
				assert.Equal(t, fixture.MachineID, response.MachineID)
				assert.NotNil(t, response.Interfaces)
				assert.Empty(t, response.Interfaces)
			}
		})
	}
}
