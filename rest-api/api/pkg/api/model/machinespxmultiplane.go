// SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
// SPDX-License-Identifier: Apache-2.0

package model

import (
	"sort"
	"strconv"
	"time"

	corev1 "github.com/NVIDIA/infra-controller/rest-api/proto/core/gen/v1"
	validation "github.com/go-ozzo/ozzo-validation/v4"
	validationis "github.com/go-ozzo/ozzo-validation/v4/is"
	"google.golang.org/protobuf/types/known/emptypb"
)

// APIMachineSpxMultiplaneSelector identifies one plane on one or all four rails.
type APIMachineSpxMultiplaneSelector struct {
	// Rail is a rail number from 0 through 3, or "all".
	Rail string `json:"rail"`
	// Plane is a plane number from 0 through 7 within each selected rail.
	Plane int `json:"plane"`
}

// Validate checks the path selectors without expanding the selected plane.
func (s APIMachineSpxMultiplaneSelector) Validate() error {
	return validation.ValidateStruct(&s,
		validation.Field(&s.Rail, validation.Required, validation.In("0", "1", "2", "3", "all")),
		validation.Field(&s.Plane, validation.Min(0), validation.Max(7)),
	)
}

// APIMachineSpxMultiplaneMaintenanceUpdateRequest changes the selected plane's maintenance mode.
type APIMachineSpxMultiplaneMaintenanceUpdateRequest struct {
	// MachineID and Selector are supplied by the URL, after binding the body.
	MachineID string                          `json:"-"`
	Selector  APIMachineSpxMultiplaneSelector `json:"-"`
	// MaintenanceMode is required: true disables the plane and false enables it.
	MaintenanceMode *bool `json:"maintenanceMode"`
	// Reference is an optional URL; omission/null preserves it and an empty string clears it.
	Reference *string `json:"reference"`
}

// ToProto preserves explicit false and the reference's preserve/set/clear semantics.
// Validate must be called on the request and selector before conversion.
func (r APIMachineSpxMultiplaneMaintenanceUpdateRequest) ToProto() *corev1.MachineSpxMultiplaneMaintenanceUpdateRequest {
	plane := uint32(r.Selector.Plane)
	request := &corev1.MachineSpxMultiplaneMaintenanceUpdateRequest{
		MachineId:       &corev1.MachineId{Id: r.MachineID},
		Plane:           &plane,
		MaintenanceMode: r.MaintenanceMode,
		Reference:       r.Reference,
	}
	if r.Selector.Rail == "all" {
		request.RailSelector = &corev1.MachineSpxMultiplaneMaintenanceUpdateRequest_AllRails{AllRails: &emptypb.Empty{}}
	} else {
		rail, _ := strconv.Atoi(r.Selector.Rail)
		request.RailSelector = &corev1.MachineSpxMultiplaneMaintenanceUpdateRequest_Rail{Rail: uint32(rail)}
	}
	return request
}

// Validate requires an explicit maintenance mode and validates any nonempty reference URL.
func (r APIMachineSpxMultiplaneMaintenanceUpdateRequest) Validate() error {
	return validation.ValidateStruct(&r,
		validation.Field(&r.MaintenanceMode, validation.NotNil.Error(validationErrorValueRequired)),
		validation.Field(&r.Reference, validationis.URL),
	)
}

const (
	// MachineSpxMultiplaneStatusPending means the requested revision has not been confirmed applied.
	MachineSpxMultiplaneStatusPending = "Pending"
	// MachineSpxMultiplaneStatusApplied means Xplane confirmed the current requested state and revision.
	MachineSpxMultiplaneStatusApplied = "Applied"
	// MachineSpxMultiplaneStatusError means applying the requested state failed.
	MachineSpxMultiplaneStatusError = "Error"
)

// APIMachineSpxMultiplane contains requested and observed maintenance state for an Astra machine.
type APIMachineSpxMultiplane struct {
	// MachineID identifies the machine whose plane states are returned.
	MachineID string `json:"machineId"`
	// Interfaces contains the declared logical interfaces and their maintenance states.
	Interfaces []APIMachineSpxMultiplaneInterface `json:"interfaces"`
}

// APIMachineSpxMultiplaneInterface contains desired and observed state for one logical interface.
type APIMachineSpxMultiplaneInterface struct {
	// LogicalIfname identifies the declared interface, such as eth_r2_p6.
	LogicalIfname string `json:"logicalIfname"`
	// Rail is the interface's rail number, from 0 through 3.
	Rail int `json:"rail"`
	// Plane is the interface's plane number, from 0 through 7.
	Plane int `json:"plane"`
	// MaintenanceMode is the requested state; all declared planes are initially in maintenance mode.
	MaintenanceMode bool `json:"maintenanceMode"`
	// Reference is the optional reference URL stored with the request.
	Reference *string `json:"reference"`
	// Revision is the version of this interface's requested state.
	Revision string `json:"revision"`
	// AppliedMaintenanceMode is null until Xplane confirms an observed state.
	AppliedMaintenanceMode *bool `json:"appliedMaintenanceMode"`
	// AppliedRevision is the revision Xplane confirmed, which may precede the requested revision.
	AppliedRevision *string `json:"appliedRevision"`
	// Netdev is the physical interface name resolved by the DPU agent, or null if unresolved.
	Netdev *string `json:"netdev"`
	// Status is Pending, Applied, or Error; Applied requires the current requested revision to match.
	Status string `json:"status"`
	// Error describes an application failure, or is null when no failure is reported.
	Error *string `json:"error"`
	// ObservedAt is the time the application state was observed, or null when not yet observed.
	ObservedAt *time.Time `json:"observedAt"`
}

// FromProto shapes the site response and reports convergence only for the current revision.
func (m *APIMachineSpxMultiplane) FromProto(p *corev1.MachineSpxMultiplane) {
	if p == nil {
		return
	}
	m.MachineID = p.GetMachineId().GetId()
	m.Interfaces = make([]APIMachineSpxMultiplaneInterface, 0, len(p.Interfaces))
	for _, state := range p.Interfaces {
		if state == nil {
			continue
		}
		entry := APIMachineSpxMultiplaneInterface{
			LogicalIfname: state.LogicalIfname, Rail: int(state.Rail), Plane: int(state.Plane),
			MaintenanceMode: state.MaintenanceMode, Reference: state.Reference, Revision: state.Revision,
			AppliedMaintenanceMode: state.AppliedMaintenanceMode, AppliedRevision: state.AppliedRevision,
			Netdev: state.Netdev, Error: state.Error, Status: MachineSpxMultiplaneStatusPending,
		}
		if state.ObservedAt != nil {
			observedAt := state.ObservedAt.AsTime()
			entry.ObservedAt = &observedAt
		}
		if state.GetError() != "" {
			entry.Status = MachineSpxMultiplaneStatusError
		} else if state.AppliedRevision != nil && state.Revision != "" && state.GetAppliedRevision() == state.Revision && state.AppliedMaintenanceMode != nil && state.GetAppliedMaintenanceMode() == state.MaintenanceMode {
			entry.Status = MachineSpxMultiplaneStatusApplied
		}
		m.Interfaces = append(m.Interfaces, entry)
	}
	sort.Slice(m.Interfaces, func(i, j int) bool {
		if m.Interfaces[i].Rail != m.Interfaces[j].Rail {
			return m.Interfaces[i].Rail < m.Interfaces[j].Rail
		}
		return m.Interfaces[i].Plane < m.Interfaces[j].Plane
	})
}
