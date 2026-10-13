// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package model

import (
	"encoding/json"
	cutil "github.com/NVIDIA/infra-controller/rest-api/common/pkg/util"
	corev1 "github.com/NVIDIA/infra-controller/rest-api/proto/core/gen/v1"
	"google.golang.org/protobuf/types/known/timestamppb"
	"strconv"
	"testing"
	"time"

	validation "github.com/go-ozzo/ozzo-validation/v4"
	"github.com/stretchr/testify/require"
)

func TestAPIMachineSpxMultiplaneSelector_Validate(t *testing.T) {
	tests := []struct {
		name     string
		selector APIMachineSpxMultiplaneSelector
		valid    bool
	}{
		{"first rail and plane", APIMachineSpxMultiplaneSelector{Rail: "0", Plane: 0}, true},
		{"last rail and plane", APIMachineSpxMultiplaneSelector{Rail: "3", Plane: 7}, true},
		{"all rails one plane", APIMachineSpxMultiplaneSelector{Rail: "all", Plane: 6}, true},
		{"rail out of range", APIMachineSpxMultiplaneSelector{Rail: "4", Plane: 6}, false},
		{"missing rail", APIMachineSpxMultiplaneSelector{Plane: 6}, false},
		{"negative plane", APIMachineSpxMultiplaneSelector{Rail: "2", Plane: -1}, false},
		{"plane out of range", APIMachineSpxMultiplaneSelector{Rail: "2", Plane: 8}, false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := tt.selector.Validate()
			if tt.valid {
				require.NoError(t, err)
			} else {
				require.Error(t, err)
				require.IsType(t, validation.Errors{}, err)
			}
		})
	}
}

func TestAPIMachineSpxMultiplaneMaintenanceUpdateRequest_Validate(t *testing.T) {
	tests := []struct {
		name  string
		body  string
		valid bool
	}{
		{"maintenance on", `{"maintenanceMode":true,"reference":"https://tickets.example.com/SPX-42"}`, true},
		{"explicit maintenance off", `{"maintenanceMode":false}`, true},
		{"missing maintenance mode", `{}`, false},
		{"null maintenance mode", `{"maintenanceMode":null}`, false},
		{"invalid reference", `{"maintenanceMode":true,"reference":"not a URL"}`, false},
		{"clear reference", `{"maintenanceMode":true,"reference":""}`, true},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			var request APIMachineSpxMultiplaneMaintenanceUpdateRequest
			require.NoError(t, json.Unmarshal([]byte(tt.body), &request))
			err := request.Validate()
			if tt.valid {
				require.NoError(t, err)
			} else {
				require.Error(t, err)
				require.IsType(t, validation.Errors{}, err)
			}
		})
	}
}

func TestAPIMachineSpxMultiplaneInterface_MarshalJSON(t *testing.T) {
	maintenanceOff := false
	revision := "V2-T1791792000000000"
	netdev := "A13p2"
	tests := []struct {
		name           string
		interfaceState APIMachineSpxMultiplaneInterface
		applied        interface{}
	}{
		{"unknown applied state", APIMachineSpxMultiplaneInterface{LogicalIfname: "eth_r2_p6", Rail: 2, Plane: 6, MaintenanceMode: true, Status: MachineSpxMultiplaneStatusPending}, nil},
		{"confirmed maintenance off", APIMachineSpxMultiplaneInterface{LogicalIfname: "eth_r2_p6", Rail: 2, Plane: 6, MaintenanceMode: false, Revision: revision, AppliedMaintenanceMode: &maintenanceOff, AppliedRevision: &revision, Netdev: &netdev, Status: MachineSpxMultiplaneStatusApplied}, false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			data, err := json.Marshal(tt.interfaceState)
			require.NoError(t, err)
			var result map[string]interface{}
			require.NoError(t, json.Unmarshal(data, &result))
			require.Contains(t, result, "maintenanceMode")
			require.Equal(t, tt.interfaceState.MaintenanceMode, result["maintenanceMode"])
			require.Contains(t, result, "appliedMaintenanceMode")
			require.Equal(t, tt.applied, result["appliedMaintenanceMode"])
			for _, field := range []string{"reference", "appliedRevision", "netdev", "error", "observedAt"} {
				require.Contains(t, result, field)
			}
		})
	}
}

func TestAPIMachineSpxMultiplaneMaintenanceUpdateRequest_ToProto(t *testing.T) {
	tests := []struct {
		name, rail  string
		maintenance bool
		reference   *string
	}{
		{"all rails with explicit false and preserved reference", "all", false, nil},
		{"rail zero with reference cleared", "0", true, cutil.GetPtr("")},
		{"last rail and reference set", "3", true, cutil.GetPtr("https://tickets.example.com/SPX-42")},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			request := APIMachineSpxMultiplaneMaintenanceUpdateRequest{MachineID: "machine-id", Selector: APIMachineSpxMultiplaneSelector{Rail: tt.rail, Plane: 6}, MaintenanceMode: &tt.maintenance, Reference: tt.reference}
			require.NoError(t, request.Validate())
			require.NoError(t, request.Selector.Validate())
			result := request.ToProto()
			require.Equal(t, "machine-id", result.GetMachineId().GetId())
			require.NotNil(t, result.Plane)
			require.Equal(t, uint32(6), result.GetPlane())
			require.NotNil(t, result.MaintenanceMode)
			require.Equal(t, tt.maintenance, result.GetMaintenanceMode())
			require.Equal(t, tt.reference, result.Reference)
			if tt.rail == "all" {
				require.NotNil(t, result.GetAllRails())
			} else {
				require.IsType(t, &corev1.MachineSpxMultiplaneMaintenanceUpdateRequest_Rail{}, result.RailSelector)
				rail, err := strconv.Atoi(tt.rail)
				require.NoError(t, err)
				require.Equal(t, uint32(rail), result.GetRail())
			}
		})
	}
}

func TestAPIMachineSpxMultiplane_FromProto(t *testing.T) {
	revision := "V2-T1791792000000000"
	tests := []struct {
		name                          string
		appliedMode                   *bool
		appliedRevision, errorMessage *string
		wantStatus                    string
	}{
		{"no observation", nil, nil, nil, MachineSpxMultiplaneStatusPending},
		{"current revision applied off", cutil.GetPtr(false), &revision, nil, MachineSpxMultiplaneStatusApplied},
		{"previous revision", cutil.GetPtr(false), cutil.GetPtr("V1-T1791791900000000"), nil, MachineSpxMultiplaneStatusPending},
		{"mode differs despite revision match", cutil.GetPtr(true), &revision, nil, MachineSpxMultiplaneStatusPending},
		{"application failed", nil, nil, cutil.GetPtr("Xplane rejected the port"), MachineSpxMultiplaneStatusError},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			observedAt := timestamppb.New(time.Date(2026, 10, 12, 12, 0, 0, 0, time.UTC))
			response := &corev1.MachineSpxMultiplane{MachineId: &corev1.MachineId{Id: "machine-id"}, Interfaces: []*corev1.MachineSpxMultiplaneInterface{
				{LogicalIfname: "eth_r2_p6", Rail: 2, Plane: 6, MaintenanceMode: false, Revision: revision, AppliedMaintenanceMode: tt.appliedMode, AppliedRevision: tt.appliedRevision, Error: tt.errorMessage, ObservedAt: observedAt},
				{LogicalIfname: "eth_r0_p1", Rail: 0, Plane: 1, MaintenanceMode: true, Revision: revision},
				{LogicalIfname: "eth_r0_p0", Rail: 0, Plane: 0, MaintenanceMode: true, Revision: revision},
			}}
			var result APIMachineSpxMultiplane
			result.FromProto(response)
			require.Equal(t, "machine-id", result.MachineID)
			require.Len(t, result.Interfaces, 3)
			require.Equal(t, "eth_r0_p0", result.Interfaces[0].LogicalIfname)
			require.Equal(t, "eth_r0_p1", result.Interfaces[1].LogicalIfname)
			entry := result.Interfaces[2]
			require.Equal(t, tt.wantStatus, entry.Status)
			require.Equal(t, tt.appliedMode, entry.AppliedMaintenanceMode)
			require.Equal(t, tt.appliedRevision, entry.AppliedRevision)
			require.Equal(t, tt.errorMessage, entry.Error)
			require.Equal(t, observedAt.AsTime(), *entry.ObservedAt)
		})
	}
	t.Run("empty interfaces serialize as an array", func(t *testing.T) {
		var result APIMachineSpxMultiplane
		result.FromProto(&corev1.MachineSpxMultiplane{MachineId: &corev1.MachineId{Id: "machine-id"}})
		data, err := json.Marshal(result)
		require.NoError(t, err)
		require.JSONEq(t, `{"machineId":"machine-id","interfaces":[]}`, string(data))
	})
}
