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

//! Desired Xplane maintenance state for the logical interfaces on an Astra host.

use std::collections::BTreeMap;

use config_version::ConfigVersion;
use serde::{Deserialize, Serialize};

use crate::expected_machine::ExpectedInterface;

/// Desired per-interface maintenance requests stored in a host's `spx_multiplane` column.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSpxMultiplane {
    /// Requests keyed by declared logical interface names, such as `eth_r2_p6`.
    pub interfaces: BTreeMap<String, MachineSpxMultiplaneInterface>,
}

impl MachineSpxMultiplane {
    /// Initial maintenance state for CX9 ports with declared canonical logical names.
    /// Hosts without these declarations do not have multiplane configuration.
    pub fn from_expected_interfaces(
        interfaces: &[ExpectedInterface],
    ) -> Result<Option<Self>, String> {
        let mut configuration = Self::default();
        for interface in interfaces {
            if interface.nic_type.as_deref() != Some("CX9") {
                continue;
            }
            let Some(name) = interface.logical_ifname.as_deref() else {
                continue;
            };
            if Self::interface_coordinates(name).is_none() {
                continue;
            }
            if configuration
                .interfaces
                .insert(name.to_string(), Default::default())
                .is_some()
            {
                return Err(format!("duplicate declared logical interface {name}"));
            }
        }
        Ok((!configuration.interfaces.is_empty()).then_some(configuration))
    }

    /// Coordinates of a canonical `eth_r<0..3>_p<0..7>` logical name.
    pub fn interface_coordinates(name: &str) -> Option<(u8, u8)> {
        let (rail, plane) = name.strip_prefix("eth_r")?.split_once("_p")?;
        let rail = rail.parse::<u8>().ok()?;
        let plane = plane.parse::<u8>().ok()?;
        (rail <= 3 && plane <= 7 && name == format!("eth_r{rail}_p{plane}"))
            .then_some((rail, plane))
    }

    /// Reconcile declarations without resetting existing operator requests.
    pub fn synchronize_declarations(&mut self, declared: Self) {
        self.interfaces
            .retain(|name, _| declared.interfaces.contains_key(name));
        for (name, initial) in declared.interfaces {
            self.interfaces.entry(name).or_insert(initial);
        }
    }
}

/// Desired maintenance state and revision for one declared Astra interface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSpxMultiplaneInterface {
    /// Whether the plane must be disabled for maintenance; new requests default to true.
    pub maintenance_mode: bool,
    /// Optional operator-provided URL describing the maintenance request.
    pub reference: Option<String>,
    /// Version of this interface's desired state, used to match application acknowledgments.
    pub revision: ConfigVersion,
    /// Last agent observation; older stored requests omit this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<MachineSpxMultiplaneObservation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSpxMultiplaneObservation {
    pub maintenance_mode: Option<bool>,
    pub revision: ConfigVersion,
    pub netdev: Option<String>,
    pub error: Option<String>,
    pub observed_at: chrono::DateTime<chrono::Utc>,
}

impl Default for MachineSpxMultiplaneInterface {
    fn default() -> Self {
        Self {
            maintenance_mode: true,
            reference: None,
            revision: ConfigVersion::initial(),
            observation: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use carbide_test_support::Outcome::{Fails, Yields};
    use carbide_test_support::{Case, check_cases};
    use serde_json::json;

    use super::*;

    #[test]
    fn declared_interfaces_initialize_and_preserve_operator_requests() {
        let interface = |name: &str| ExpectedInterface {
            logical_ifname: Some(name.to_string()),
            cerebro_ifname: Some("C1-1-L1".to_string()),
            nic_type: Some("CX9".to_string()),
            ..Default::default()
        };
        let declaration = interface("eth_r2_p6");
        let mut configuration =
            MachineSpxMultiplane::from_expected_interfaces(std::slice::from_ref(&declaration))
                .unwrap()
                .unwrap();
        assert!(configuration.interfaces["eth_r2_p6"].maintenance_mode);
        configuration
            .interfaces
            .get_mut("eth_r2_p6")
            .unwrap()
            .maintenance_mode = false;
        let requested_revision = configuration.interfaces["eth_r2_p6"].revision;
        configuration.synchronize_declarations(
            MachineSpxMultiplane::from_expected_interfaces(&[
                declaration.clone(),
                interface("eth_r0_p0"),
            ])
            .unwrap()
            .unwrap(),
        );
        assert!(!configuration.interfaces["eth_r2_p6"].maintenance_mode);
        assert_eq!(
            configuration.interfaces["eth_r2_p6"].revision,
            requested_revision
        );
        assert!(configuration.interfaces["eth_r0_p0"].maintenance_mode);
        assert!(
            MachineSpxMultiplane::from_expected_interfaces(&[declaration.clone(), declaration])
                .is_err()
        );
        assert!(
            MachineSpxMultiplane::from_expected_interfaces(&[
                ExpectedInterface::default(),
                interface("other0")
            ])
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn new_interface_starts_in_maintenance() {
        let interface = MachineSpxMultiplaneInterface::default();
        assert!(interface.maintenance_mode);
        assert!(interface.reference.is_none());
        assert_eq!(interface.revision.version_nr(), 1);
    }

    #[test]
    fn interface_json_requires_explicit_maintenance_mode_and_preserves_requests() {
        let revision = ConfigVersion::new(7);
        check_cases(
            [
                Case {
                    scenario: "maintenance on with reference",
                    input: json!({
                        "maintenance_mode": true,
                        "reference": "https://example.com/ticket/123",
                        "revision": revision,
                    }),
                    expect: Yields(MachineSpxMultiplaneInterface {
                        maintenance_mode: true,
                        reference: Some("https://example.com/ticket/123".to_string()),
                        revision,
                        observation: None,
                    }),
                },
                Case {
                    scenario: "explicit maintenance off",
                    input: json!({"maintenance_mode": false, "reference": null, "revision": revision}),
                    expect: Yields(MachineSpxMultiplaneInterface {
                        maintenance_mode: false,
                        reference: None,
                        revision,
                        observation: None,
                    }),
                },
                Case {
                    scenario: "missing maintenance must not enable a plane",
                    input: json!({"reference": null, "revision": revision}),
                    expect: Fails,
                },
            ],
            |input| {
                serde_json::from_value::<MachineSpxMultiplaneInterface>(input)
                    .and_then(|request| {
                        // Exercise the persisted JSON representation in both directions.
                        serde_json::from_value(serde_json::to_value(request)?)
                    })
                    .map_err(drop)
            },
        );
    }
}
