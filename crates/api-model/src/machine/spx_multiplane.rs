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

/// Desired per-interface maintenance requests stored in a host's `spx_multiplane` column.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineSpxMultiplane {
    /// Requests keyed by declared logical interface names, such as `eth_r2_p6`.
    pub interfaces: BTreeMap<String, MachineSpxMultiplaneInterface>,
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
}

impl Default for MachineSpxMultiplaneInterface {
    fn default() -> Self {
        Self {
            maintenance_mode: true,
            reference: None,
            revision: ConfigVersion::initial(),
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
                    }),
                },
                Case {
                    scenario: "explicit maintenance off",
                    input: json!({"maintenance_mode": false, "reference": null, "revision": revision}),
                    expect: Yields(MachineSpxMultiplaneInterface {
                        maintenance_mode: false,
                        reference: None,
                        revision,
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
