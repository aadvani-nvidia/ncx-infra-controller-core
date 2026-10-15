/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 * http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Requested maintenance state for Spectrum-X planes.

mod args;

pub(crate) use args::Args;
use args::{Action, MaintenanceMode, RailSelector};
use prettytable::{Table, row};
use rpc::admin_cli::OutputFormat;
use rpc::forge;

use crate::cfg::dispatch::dispatch_via_run;
use crate::cfg::run::Run;
use crate::cfg::runtime::RuntimeContext;
use crate::errors::CarbideCliResult;

impl Run for Args {
    async fn run(self, ctx: &mut RuntimeContext) -> CarbideCliResult<()> {
        let configuration = match self.action {
            Action::Show => {
                tracing::info!(machine_id = %self.machine_id, "CLI requesting SPX maintenance state from Core");
                ctx.api_client
                    .0
                    .get_machine_spx_multiplane(forge::MachineSpxMultiplaneRequest {
                        machine_id: Some(self.machine_id),
                    })
                    .await?
            }
            Action::Multiplane {
                rail,
                plane,
                mode,
                reference,
                ..
            } => {
                tracing::info!(machine_id = %self.machine_id, ?rail, plane, maintenance_mode = mode == MaintenanceMode::On, reference_supplied = reference.is_some(), "CLI sending SPX maintenance update to Core");
                let rail_selector = match rail {
                    RailSelector::One(rail) => forge::machine_spx_multiplane_maintenance_update_request::RailSelector::Rail(rail.into()),
                    RailSelector::All => forge::machine_spx_multiplane_maintenance_update_request::RailSelector::AllRails(()),
                };
                ctx.api_client
                    .0
                    .update_machine_spx_multiplane_maintenance(
                        forge::MachineSpxMultiplaneMaintenanceUpdateRequest {
                            machine_id: Some(self.machine_id),
                            rail_selector: Some(rail_selector),
                            plane: Some(plane.into()),
                            maintenance_mode: Some(mode == MaintenanceMode::On),
                            reference,
                        },
                    )
                    .await?
            }
        };
        tracing::info!(machine_id = %self.machine_id, interface_count = configuration.interfaces.len(), "CLI received SPX maintenance state from Core");
        if ctx.config.format == OutputFormat::Json {
            println!("{}", serde_json::to_string_pretty(&configuration)?);
        } else {
            let mut table = Table::new();
            table.set_titles(row![
                "Logical interface",
                "Rail",
                "Plane",
                "Requested maintenance mode",
                "Reference",
                "Revision",
                "Applied maintenance mode",
                "Status",
                "Netdev",
                "Error"
            ]);
            for state in configuration.interfaces {
                let status = if state.observed_at.is_none() {
                    "Pending"
                } else if state.error.is_some() && state.applied_revision.is_none() {
                    "Error"
                } else if state.applied_revision.as_deref() == Some(&state.revision)
                    && state.applied_maintenance_mode == Some(state.maintenance_mode)
                {
                    "Applied"
                } else {
                    "Pending"
                };
                table.add_row(row![
                    state.logical_ifname,
                    state.rail,
                    state.plane,
                    if state.maintenance_mode { "on" } else { "off" },
                    state.reference.as_deref().unwrap_or_default(),
                    state.revision,
                    match state.applied_maintenance_mode {
                        Some(true) => "on",
                        Some(false) => "off",
                        None => "unknown",
                    },
                    status,
                    state.netdev.as_deref().unwrap_or_default(),
                    state.error.as_deref().unwrap_or_default()
                ]);
            }
            table.printstd();
        }
        Ok(())
    }
}

dispatch_via_run!(Args);
