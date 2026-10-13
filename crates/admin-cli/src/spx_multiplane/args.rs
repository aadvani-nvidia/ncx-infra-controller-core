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

use std::str::FromStr;

use carbide_uuid::machine::MachineId;
use clap::{Parser, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    about = "Control maintenance mode for Spectrum-X planes on a machine",
    override_usage = "nico-admin-cli spx-multiplane --machine-id <MACHINE_ID> maintenance-mode <on|off> --rail <0-3|all> --plane <0-7> [--reference <URL>]\n       nico-admin-cli spx-multiplane --machine-id <MACHINE_ID> show",
    long_about = "Control maintenance mode for Spectrum-X planes on a machine. Each rail has \
        two SuperNICs: planes 0-3 belong to one, and planes 4-7 belong to the other. \
        Rail R and plane P select the specified rail and plane that are affected;
        its matching plane on the other SuperNIC is unchanged. All planes start \
        in maintenance mode. Maintenance mode on disables a plane; maintenance mode off enables it.",
    after_long_help = "\
EXAMPLES:

Disable plane 6 on rail 2 (eth_r2_p6):
    $ nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 \
    maintenance-mode on --rail 2 --plane 6 --reference https://tickets.example.com/SPX-42

Enable plane 6 on rail 2:
    $ nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 \
    maintenance-mode off --rail 2 --plane 6

Disable plane 6 on all four rails:
    $ nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 \
    maintenance-mode on --rail all --plane 6

Show plane maintenance state for a machine:
    $ nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 show

"
)]
pub(crate) struct Args {
    #[arg(long, value_name = "MACHINE_ID", help = "Machine ID of the host")]
    pub(super) machine_id: MachineId,
    #[command(subcommand)]
    pub(super) action: Action,
}

#[derive(Parser, Debug)]
pub(super) enum Action {
    #[command(
        about = "Turn maintenance mode on (disable) or off (enable) for the selected plane",
        override_usage = "nico-admin-cli spx-multiplane --machine-id <MACHINE_ID> maintenance-mode <on|off> --rail <0-3|all> --plane <0-7> [--reference <URL>]",
        after_long_help = "\
EXAMPLES:

Disable plane 6 on rail 2 with a ticket reference:
    $ nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 \
    maintenance-mode on --rail 2 --plane 6 --reference https://tickets.example.com/SPX-42

Enable plane 6 on rail 2:
    $ nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 \
    maintenance-mode off --rail 2 --plane 6

Disable plane 6 on all four rails without changing plane 2:
    $ nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 \
    maintenance-mode on --rail all --plane 6

"
    )]
    MaintenanceMode {
        #[arg(value_enum, help = "on disables the plane; off enables it")]
        mode: MaintenanceMode,
        #[arg(
            long,
            value_name = "RAIL",
            help = "Rail number (0-3), or all four rails"
        )]
        rail: RailSelector,
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=7), help = "Plane number (0-7); planes 0-3 and 4-7 belong to separate SuperNICs")]
        plane: u8,
        #[arg(
            long,
            value_name = "URL",
            help = "Optional URL describing the reason for the change"
        )]
        reference: Option<String>,
    },
    #[command(
        about = "Show requested and applied plane maintenance state",
        override_usage = "nico-admin-cli spx-multiplane --machine-id <MACHINE_ID> show",
        after_long_help = "\
EXAMPLES:

Show plane maintenance state for a machine:
    $ nico-admin-cli spx-multiplane --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 show

"
    )]
    Show,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RailSelector {
    One(u8),
    All,
}

impl FromStr for RailSelector {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value == "all" {
            return Ok(Self::All);
        }
        match value.parse::<u8>() {
            Ok(rail @ 0..=3) => Ok(Self::One(rail)),
            _ => Err("rail must be 0, 1, 2, 3, or all".to_string()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(super) enum MaintenanceMode {
    On,
    Off,
}

#[cfg(test)]
mod tests {
    use carbide_test_support::Outcome::Yields;
    use carbide_test_support::{Case, check_cases};
    use clap::Parser;
    use clap::error::ErrorKind;

    use super::*;
    use crate::cfg::cli_options::{CliCommand, CliOptions};

    const MACHINE: &str = "fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80";

    fn parse(arguments: &str) -> Result<Args, ErrorKind> {
        let options = CliOptions::try_parse_from(
            ["nico-admin-cli", "spx-multiplane"]
                .into_iter()
                .chain(arguments.split_whitespace()),
        )
        .map_err(|error| error.kind())?;
        match options.commands {
            Some(CliCommand::SpxMultiplane(args)) => Ok(args),
            _ => panic!("public CLI did not route to spx-multiplane"),
        }
    }

    #[test]
    fn public_command_preserves_selected_rail_plane_and_mode() {
        check_cases(
            [
                Case {
                    scenario: "first rail and plane, maintenance on",
                    input: "maintenance-mode on --rail 0 --plane 0",
                    expect: Yields((RailSelector::One(0), 0, MaintenanceMode::On, None)),
                },
                Case {
                    scenario: "last rail and plane, maintenance off with reference",
                    input: "maintenance-mode off --rail 3 --plane 7 --reference https://tickets.example.com/SPX-42",
                    expect: Yields((
                        RailSelector::One(3),
                        7,
                        MaintenanceMode::Off,
                        Some("https://tickets.example.com/SPX-42".to_string()),
                    )),
                },
                Case {
                    scenario: "all rails, only specified plane",
                    input: "maintenance-mode on --rail all --plane 6",
                    expect: Yields((RailSelector::All, 6, MaintenanceMode::On, None)),
                },
            ],
            |selection| {
                let args = parse(&format!("--machine-id {MACHINE} {selection}"))?;
                assert_eq!(args.machine_id, MACHINE.parse::<MachineId>().unwrap());
                match args.action {
                    Action::MaintenanceMode {
                        rail,
                        plane,
                        mode,
                        reference,
                    } => Ok::<_, ErrorKind>((rail, plane, mode, reference)),
                    Action::Show => panic!("maintenance invocation routed to show"),
                }
            },
        );
    }

    #[test]
    fn public_command_rejects_invalid_selectors_and_incomplete_invocations() {
        check_cases(
            [
                Case {
                    scenario: "rail outside 0-3",
                    input: format!("--machine-id {MACHINE} maintenance-mode on --rail 4 --plane 6"),
                    expect: Yields(ErrorKind::ValueValidation),
                },
                Case {
                    scenario: "plane outside 0-7",
                    input: format!("--machine-id {MACHINE} maintenance-mode on --rail 2 --plane 8"),
                    expect: Yields(ErrorKind::ValueValidation),
                },
                Case {
                    scenario: "invalid maintenance mode",
                    input: format!(
                        "--machine-id {MACHINE} maintenance-mode true --rail 2 --plane 6"
                    ),
                    expect: Yields(ErrorKind::InvalidValue),
                },
                Case {
                    scenario: "missing machine",
                    input: "maintenance-mode on --rail 2 --plane 6".to_string(),
                    expect: Yields(ErrorKind::MissingRequiredArgument),
                },
                Case {
                    scenario: "missing maintenance mode",
                    input: format!("--machine-id {MACHINE} maintenance-mode --rail 2 --plane 6"),
                    expect: Yields(ErrorKind::MissingRequiredArgument),
                },
                Case {
                    scenario: "missing rail selector",
                    input: format!("--machine-id {MACHINE} maintenance-mode on --plane 6"),
                    expect: Yields(ErrorKind::MissingRequiredArgument),
                },
                Case {
                    scenario: "missing plane selector",
                    input: format!("--machine-id {MACHINE} maintenance-mode on --rail 2"),
                    expect: Yields(ErrorKind::MissingRequiredArgument),
                },
                Case {
                    scenario: "invalid machine ID",
                    input: "--machine-id not-a-machine maintenance-mode on --rail 2 --plane 6"
                        .to_string(),
                    expect: Yields(ErrorKind::ValueValidation),
                },
            ],
            |arguments| {
                Ok::<_, ()>(parse(&arguments).expect_err("invalid invocation was accepted"))
            },
        );
    }

    #[test]
    fn maintenance_help_includes_machine_and_both_selectors() {
        let error = CliOptions::try_parse_from([
            "nico-admin-cli",
            "spx-multiplane",
            "--machine-id",
            MACHINE,
            "maintenance-mode",
            "--help",
        ])
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DisplayHelp);
        let help = error.to_string();
        assert!(help.contains(
            "--machine-id <MACHINE_ID> maintenance-mode <on|off> --rail <0-3|all> --plane <0-7>"
        ));
        assert!(help.contains("EXAMPLES:"));
    }

    #[test]
    fn show_parses_for_selected_machine() {
        let args = parse(&format!("--machine-id {MACHINE} show")).unwrap();
        assert!(matches!(args.action, Action::Show));
        assert_eq!(args.machine_id, MACHINE.parse::<MachineId>().unwrap());
    }
}
