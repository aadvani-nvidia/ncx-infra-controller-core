/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use carbide_uuid::machine::{HostMachineId, MachineId};
use model::machine::machine_search_config::MachineSearchConfig;
use model::machine::spx_multiplane::{MachineSpxMultiplane, MachineSpxMultiplaneObservation};
use rpc::forge;
use sqlx::PgConnection;
use tonic::{Request, Response, Status};

use crate::CarbideError;
use crate::api::{Api, log_request_data};

/// Initialize older hosts and reconcile expected declarations on the next config
/// poll. This runs independently of discovered NICs and attachment state.
pub(super) async fn load_configuration(
    txn: &mut PgConnection,
    machine_id: HostMachineId,
) -> Result<Option<MachineSpxMultiplane>, CarbideError> {
    let previous = db::machine::lock_spx_multiplane(txn, &machine_id).await?;
    let machine = db::machine::find_one(&mut *txn, &machine_id, MachineSearchConfig::default())
        .await?
        .ok_or_else(|| CarbideError::NotFoundError {
            kind: "machine",
            id: machine_id.to_string(),
        })?;
    let Some(bmc_mac) = machine.status.bmc_info.mac else {
        tracing::info!(%machine_id, "SPX maintenance configuration unavailable: host has no BMC MAC");
        return Ok(None);
    };
    let Some(expected) = db::expected_machine::find_by_bmc_mac_address(&mut *txn, bmc_mac).await?
    else {
        tracing::info!(%machine_id, "SPX maintenance configuration unavailable: expected machine not found");
        return Ok(None);
    };
    let declared = MachineSpxMultiplane::from_expected_interfaces(&expected.data.interfaces)
        .map_err(CarbideError::InvalidArgument)?;
    let Some(declared) = declared else {
        tracing::info!(%machine_id, "SPX maintenance configuration unavailable: no declared CX9 logical interfaces");
        return Ok(None);
    };
    let mut configuration = previous.clone().unwrap_or_default();
    configuration.synchronize_declarations(declared);
    if previous.as_ref() != Some(&configuration) {
        tracing::info!(%machine_id, interface_count = configuration.interfaces.len(), initializing = previous.is_none(), "Staging SPX maintenance declarations");
        db::machine::update_spx_multiplane(txn, &machine_id, &configuration).await?;
    }
    Ok(Some(configuration))
}

fn host_id(id: Option<MachineId>) -> Result<HostMachineId, CarbideError> {
    id.ok_or(CarbideError::MissingArgument("machine_id"))?
        .try_into()
        .map_err(|_| CarbideError::InvalidArgument("machine_id must identify a host".to_string()))
}

fn response(id: HostMachineId, configuration: MachineSpxMultiplane) -> forge::MachineSpxMultiplane {
    forge::MachineSpxMultiplane {
        machine_id: Some(id.into()),
        interfaces: configuration
            .interfaces
            .into_iter()
            .map(|(name, state)| {
                let (rail, plane) = MachineSpxMultiplane::interface_coordinates(&name)
                    .expect("configuration contains canonical declared logical interfaces");
                forge::MachineSpxMultiplaneInterface {
                    logical_ifname: name,
                    rail: rail.into(),
                    plane: plane.into(),
                    maintenance_mode: state.maintenance_mode,
                    reference: state.reference,
                    revision: state.revision.to_string(),
                    applied_maintenance_mode: state
                        .observation
                        .as_ref()
                        .and_then(|o| o.maintenance_mode),
                    applied_revision: state
                        .observation
                        .as_ref()
                        .filter(|o| o.error.is_none() && o.maintenance_mode.is_some())
                        .map(|o| o.revision.to_string()),
                    netdev: state.observation.as_ref().and_then(|o| o.netdev.clone()),
                    error: state
                        .observation
                        .as_ref()
                        .filter(|o| o.revision == state.revision)
                        .and_then(|o| o.error.clone()),
                    observed_at: state.observation.as_ref().map(|o| o.observed_at.into()),
                }
            })
            .collect(),
    }
}

/// Store readback independently of attachment discovery. Ignore reports for an
/// older desired revision so a delayed status cannot acknowledge a newer request.
pub(super) async fn record_observations(
    txn: &mut PgConnection,
    id: HostMachineId,
    observations: &[forge::AstraMultiplaneInterfaceStatus],
) -> Result<(), CarbideError> {
    if observations.is_empty() {
        return Ok(());
    }
    tracing::info!(machine_id = %id, reported_interface_count = observations.len(), "Processing SPX maintenance observations");
    let Some(mut configuration) = db::machine::lock_spx_multiplane(txn, &id).await? else {
        tracing::info!(machine_id = %id, "Ignoring SPX maintenance observations: no stored configuration");
        return Ok(());
    };
    let mut changed = false;
    for report in observations {
        let Some(state) = configuration.interfaces.get_mut(&report.logical_ifname) else {
            tracing::info!(machine_id = %id, logical_ifname = %report.logical_ifname, revision = %report.revision, "Ignoring SPX maintenance observation: interface is not declared");
            continue;
        };
        if report.revision != state.revision.to_string() {
            tracing::info!(machine_id = %id, logical_ifname = %report.logical_ifname, reported_revision = %report.revision, requested_revision = %state.revision, "Ignoring SPX maintenance observation: revision does not match current request");
            continue;
        }
        let Some(observed_at) = report
            .observed_at
            .and_then(|t| chrono::DateTime::<chrono::Utc>::try_from(t).ok())
        else {
            tracing::info!(machine_id = %id, logical_ifname = %report.logical_ifname, revision = %report.revision, "Ignoring SPX maintenance observation: timestamp is missing or invalid");
            continue;
        };
        if state
            .observation
            .as_ref()
            .is_some_and(|o| o.revision == state.revision && o.observed_at >= observed_at)
        {
            tracing::info!(machine_id = %id, logical_ifname = %report.logical_ifname, revision = %report.revision, %observed_at, "Ignoring SPX maintenance observation: timestamp is not newer");
            continue;
        }
        let error = report.error.clone().or_else(|| {
            (report.maintenance_mode != Some(state.maintenance_mode))
                .then(|| "Xplane readback does not match requested maintenance mode".to_string())
        });
        tracing::info!(machine_id = %id, logical_ifname = %report.logical_ifname, netdev = ?report.netdev, revision = %report.revision, requested_maintenance_mode = state.maintenance_mode, observed_maintenance_mode = ?report.maintenance_mode, ?error, %observed_at, "Staging SPX maintenance observation");
        state.observation = Some(MachineSpxMultiplaneObservation {
            maintenance_mode: report.maintenance_mode,
            revision: state.revision,
            netdev: report.netdev.clone(),
            error,
            observed_at,
        });
        changed = true;
    }
    if changed {
        db::machine::update_spx_multiplane(txn, &id, &configuration).await?;
        tracing::info!(machine_id = %id, "SPX maintenance observations written in transaction");
    }
    Ok(())
}

fn require_configuration(
    configuration: Option<MachineSpxMultiplane>,
) -> Result<MachineSpxMultiplane, CarbideError> {
    configuration.ok_or_else(|| {
        CarbideError::FailedPrecondition(
            "host has no declared CX9 logical interfaces for SPX multiplane maintenance"
                .to_string(),
        )
    })
}

pub(crate) async fn get(
    api: &Api,
    request: Request<forge::MachineSpxMultiplaneRequest>,
) -> Result<Response<forge::MachineSpxMultiplane>, Status> {
    log_request_data(&request);
    let id = host_id(request.into_inner().machine_id)?;
    tracing::info!(machine_id = %id, "Reading SPX maintenance state");
    let mut txn = api.txn_begin().await?;
    let configuration = require_configuration(load_configuration(&mut txn, id).await?)?;
    txn.commit().await?;
    tracing::info!(machine_id = %id, interface_count = configuration.interfaces.len(), "Returning SPX maintenance state");
    Ok(Response::new(response(id, configuration)))
}

pub(crate) async fn update(
    api: &Api,
    request: Request<forge::MachineSpxMultiplaneMaintenanceUpdateRequest>,
) -> Result<Response<forge::MachineSpxMultiplane>, Status> {
    log_request_data(&request);
    let request = request.into_inner();
    let id = host_id(request.machine_id)?;
    tracing::info!(machine_id = %id, rail_selector = ?request.rail_selector, plane = ?request.plane, maintenance_mode = ?request.maintenance_mode, reference_supplied = request.reference.is_some(), "Received SPX maintenance update request");
    let plane = request
        .plane
        .ok_or(CarbideError::MissingArgument("plane"))?;
    if plane > 7 {
        return Err(
            CarbideError::InvalidArgument("plane must be from 0 through 7".to_string()).into(),
        );
    }
    let maintenance_mode = request
        .maintenance_mode
        .ok_or(CarbideError::MissingArgument("maintenance_mode"))?;
    let rails: Vec<u32> = match request.rail_selector {
        Some(forge::machine_spx_multiplane_maintenance_update_request::RailSelector::Rail(
            rail,
        )) if rail <= 3 => vec![rail],
        Some(forge::machine_spx_multiplane_maintenance_update_request::RailSelector::AllRails(
            _,
        )) => (0..=3).collect(),
        Some(_) => {
            return Err(
                CarbideError::InvalidArgument("rail must be from 0 through 3".to_string()).into(),
            );
        }
        None => return Err(CarbideError::MissingArgument("rail_selector").into()),
    };
    if let Some(reference) = request.reference.as_ref().filter(|value| !value.is_empty()) {
        url::Url::parse(reference).map_err(|_| {
            CarbideError::InvalidArgument("reference must be a URL or an empty string".to_string())
        })?;
    }
    let names: Vec<String> = rails
        .iter()
        .map(|rail| format!("eth_r{rail}_p{plane}"))
        .collect();
    let mut txn = api.txn_begin().await?;
    let mut configuration = require_configuration(load_configuration(&mut txn, id).await?)?;
    // Validate the full selection before mutating any entry. A missing rail
    // rejects the whole request, including initialization in this transaction.
    for name in &names {
        if !configuration.interfaces.contains_key(name) {
            tracing::info!(machine_id = %id, logical_ifname = %name, "Rejecting SPX maintenance update: selected interface is not declared");
            return Err(CarbideError::InvalidArgument(format!(
                "logical interface {name} is not declared on the expected machine"
            ))
            .into());
        }
    }
    for name in names {
        let state = configuration
            .interfaces
            .get_mut(&name)
            .expect("selection was validated");
        let reference = match &request.reference {
            None => state.reference.clone(),
            Some(value) if value.is_empty() => None,
            Some(value) => Some(value.clone()),
        };
        if state.maintenance_mode != maintenance_mode || state.reference != reference {
            tracing::info!(machine_id = %id, logical_ifname = %name, previous_maintenance_mode = state.maintenance_mode, requested_maintenance_mode = maintenance_mode, previous_revision = %state.revision, reference_changed = state.reference != reference, "Updating SPX maintenance request in transaction");
            state.maintenance_mode = maintenance_mode;
            state.reference = reference;
            state.revision = state.revision.increment();
            tracing::info!(machine_id = %id, logical_ifname = %name, maintenance_mode = state.maintenance_mode, revision = %state.revision, "SPX maintenance request staged");
        } else {
            tracing::info!(machine_id = %id, logical_ifname = %name, maintenance_mode = state.maintenance_mode, revision = %state.revision, "SPX maintenance request already matches; preserving revision");
        }
    }
    db::machine::update_spx_multiplane(&mut txn, &id, &configuration).await?;
    txn.commit().await?;
    tracing::info!(machine_id = %id, ?rails, plane, maintenance_mode, selected_interface_count = rails.len(), "Committed SPX maintenance update");
    Ok(Response::new(response(id, configuration)))
}
