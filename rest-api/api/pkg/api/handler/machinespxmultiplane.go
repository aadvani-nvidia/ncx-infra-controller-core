// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package handler

import (
	"errors"
	"net/http"
	"strconv"

	"github.com/labstack/echo/v4"
	"google.golang.org/protobuf/proto"

	"github.com/NVIDIA/infra-controller/rest-api/api/pkg/api/handler/util/common"
	"github.com/NVIDIA/infra-controller/rest-api/api/pkg/api/model"
	sc "github.com/NVIDIA/infra-controller/rest-api/api/pkg/client/site"
	cutil "github.com/NVIDIA/infra-controller/rest-api/common/pkg/util"
	cdb "github.com/NVIDIA/infra-controller/rest-api/db/pkg/db"
	cdbm "github.com/NVIDIA/infra-controller/rest-api/db/pkg/db/model"
	corev1 "github.com/NVIDIA/infra-controller/rest-api/proto/core/gen/v1"
)

const (
	getMachineSpxMultiplaneMethod               = "/forge.Forge/GetMachineSpxMultiplane"
	updateMachineSpxMultiplaneMaintenanceMethod = "/forge.Forge/UpdateMachineSpxMultiplaneMaintenance"
)

type machineSpxMultiplaneHandler struct {
	dbSession *cdb.Session
	scp       *sc.ClientPool
}

// GetMachineSpxMultiplaneHandler reads requested and observed plane maintenance state.
type GetMachineSpxMultiplaneHandler struct{ machineSpxMultiplaneHandler }

// NewGetMachineSpxMultiplaneHandler returns the handler for a machine's plane state.
func NewGetMachineSpxMultiplaneHandler(dbSession *cdb.Session, scp *sc.ClientPool) GetMachineSpxMultiplaneHandler {
	return GetMachineSpxMultiplaneHandler{machineSpxMultiplaneHandler{dbSession: dbSession, scp: scp}}
}

// Handle reads the site-authoritative state through the shared Core proxy.
func (h GetMachineSpxMultiplaneHandler) Handle(c echo.Context) error { return h.handle(c, false) }

// UpdateMachineSpxMultiplaneMaintenanceHandler changes one plane on selected rails.
type UpdateMachineSpxMultiplaneMaintenanceHandler struct{ machineSpxMultiplaneHandler }

// NewUpdateMachineSpxMultiplaneMaintenanceHandler returns the maintenance update handler.
func NewUpdateMachineSpxMultiplaneMaintenanceHandler(dbSession *cdb.Session, scp *sc.ClientPool) UpdateMachineSpxMultiplaneMaintenanceHandler {
	return UpdateMachineSpxMultiplaneMaintenanceHandler{machineSpxMultiplaneHandler{dbSession: dbSession, scp: scp}}
}

// Handle validates a maintenance update and returns the saved desired state.
func (h UpdateMachineSpxMultiplaneMaintenanceHandler) Handle(c echo.Context) error {
	return h.handle(c, true)
}

func (h machineSpxMultiplaneHandler) handle(c echo.Context, update bool) error {
	action := "Get"
	if update {
		action = "UpdateMaintenance"
	}
	org, dbUser, ctx, logger, handlerSpan := common.SetupHandler("MachineSpxMultiplane", action, c)
	if handlerSpan != nil {
		defer handlerSpan.End()
	}
	if dbUser == nil {
		return cutil.NewAPIErrorResponse(c, http.StatusInternalServerError, "Failed to retrieve current user", nil)
	}
	machineID := c.Param("id")
	if machineID == "" {
		return cutil.NewAPIErrorResponse(c, http.StatusBadRequest, "Machine ID was not specified in URL", nil)
	}

	var coreRequest proto.Message = &corev1.MachineSpxMultiplaneRequest{MachineId: &corev1.MachineId{Id: machineID}}
	fullMethod := getMachineSpxMultiplaneMethod
	if update {
		var request model.APIMachineSpxMultiplaneMaintenanceUpdateRequest
		if err := c.Bind(&request); err != nil {
			return cutil.NewAPIErrorResponse(c, http.StatusBadRequest, "Invalid request body", nil)
		}
		if err := request.Validate(); err != nil {
			return cutil.NewAPIErrorResponse(c, http.StatusBadRequest, err.Error(), nil)
		}
		plane, err := strconv.Atoi(c.Param("plane"))
		if err != nil {
			return cutil.NewAPIErrorResponse(c, http.StatusBadRequest, "plane must be an integer from 0 through 7", nil)
		}
		// Path values are authoritative; body fields cannot replace the selectors.
		request.MachineID = machineID
		request.Selector = model.APIMachineSpxMultiplaneSelector{Rail: c.Param("rail"), Plane: plane}
		if err := request.Selector.Validate(); err != nil {
			return cutil.NewAPIErrorResponse(c, http.StatusBadRequest, err.Error(), nil)
		}
		coreRequest = request.ToProto()
		fullMethod = updateMachineSpxMultiplaneMaintenanceMethod
	}

	provider, apiErr := common.IsProvider(ctx, logger, h.dbSession, org, dbUser, !update)
	if apiErr != nil {
		return cutil.NewAPIErrorResponse(c, apiErr.Code, apiErr.Message, apiErr.Data)
	}
	machine, err := cdbm.NewMachineDAO(h.dbSession).GetByID(ctx, nil, machineID, []string{cdbm.SiteRelationName}, false)
	if err != nil {
		if errors.Is(err, cdb.ErrDoesNotExist) {
			return cutil.NewAPIErrorResponse(c, http.StatusNotFound, "Could not find Machine with specified ID", nil)
		}
		logger.Error().Err(err).Msg("failed to retrieve Machine details from DB")
		return cutil.NewAPIErrorResponse(c, http.StatusInternalServerError, "Failed to retrieve Machine details, DB error", nil)
	}
	if machine.InfrastructureProviderID != provider.ID {
		return cutil.NewAPIErrorResponse(c, http.StatusNotFound, "Could not find Machine with specified ID", nil)
	}
	if machine.IsMissingOnSite {
		return cutil.NewAPIErrorResponse(c, http.StatusPreconditionFailed, "Machine is missing on site", nil)
	}
	// Provider Admins may change maintenance mode while an Instance is attached.
	if machine.Site == nil {
		return cutil.NewAPIErrorResponse(c, http.StatusInternalServerError, "Failed to retrieve Site details for Machine, DB error", nil)
	}
	if machine.Site.Status != cdbm.SiteStatusRegistered {
		return cutil.NewAPIErrorResponse(c, http.StatusPreconditionFailed, "Site is not in Registered state", nil)
	}
	stc, err := h.scp.GetClientByID(machine.SiteID)
	if err != nil {
		logger.Error().Err(err).Msg("failed to retrieve Temporal client for Site")
		return cutil.NewAPIErrorResponse(c, http.StatusInternalServerError, "Failed to retrieve workflow client for Site", nil)
	}

	// Core owns declared-interface validation, atomic rail-all updates, and revisions.
	var coreResponse corev1.MachineSpxMultiplane
	if apiErr := common.ExecuteCoreGRPC(ctx, stc, fullMethod, coreRequest, &coreResponse, machine.SiteID.String()); apiErr != nil {
		logAPIError(logger, apiErr, "failed to access SPX multiplane state via Core proxy")
		return cutil.NewAPIErrorResponse(c, apiErr.Code, apiErr.Message, nil)
	}
	if coreResponse.GetMachineId().GetId() != machineID {
		logger.Error().Msg("Core returned SPX multiplane state for an unexpected Machine")
		return cutil.NewAPIErrorResponse(c, http.StatusInternalServerError, "Invalid SPX multiplane response from Site", nil)
	}
	var response model.APIMachineSpxMultiplane
	response.FromProto(&coreResponse)
	return c.JSON(http.StatusOK, response)
}
