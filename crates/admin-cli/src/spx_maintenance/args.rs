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
    about = "Configure maintenance mode for Spectrum-X planes on a machine",
    override_usage = "nico-admin-cli spx-maintenance --machine-id <MACHINE_ID> multiplane rail <0-3|all> plane <0-7> <on|off> [--reference <REFERENCE>]\n       nico-admin-cli spx-maintenance --machine-id <MACHINE_ID> show",
    long_about = "Configure maintenance mode for Spectrum-X planes on a machine. Each rail has \
        two SuperNICs: planes 0-3 belong to one, and planes 4-7 belong to the other. \
        Rail R and plane P select the specified rail and plane that are affected;
        its matching plane on the other SuperNIC is unchanged. The DPU flavor initializes \
        planes in maintenance mode. Until a CLI request is made, the agent observes without changing that state. Maintenance mode on disables a plane; maintenance mode off enables it.",
    after_long_help = "\
EXAMPLES:

Enable maintenance mode on plane 6 on rail 2 (eth_r2_p6):
    $ nico-admin-cli spx-maintenance --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 \
    multiplane rail 2 plane 6 on --reference SPX-42

Disable maintenance mode on plane 6 on rail 2:
    $ nico-admin-cli spx-maintenance --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 \
    multiplane rail 2 plane 6 off

Enable maintenance mode plane 6 on all four rails:
    $ nico-admin-cli spx-maintenance --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 \
    multiplane rail all plane 6 on

Show host requests and per-DPU maintenance state for a machine:
    $ nico-admin-cli spx-maintenance --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 show

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
        about = "Turn maintenance mode on or off for the selected plane",
        override_usage = "nico-admin-cli spx-maintenance --machine-id <MACHINE_ID> multiplane rail <0-3|all> plane <0-7> <on|off> [--reference <REFERENCE>]",
        after_long_help = "\
EXAMPLES:

Enable maintenance mode on plane 6 on rail 2 with a ticket reference:
    $ nico-admin-cli spx-maintenance --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 \
    multiplane rail 2 plane 6 on --reference SPX-42

Disable maintenance mode on plane 6 on rail 2:
    $ nico-admin-cli spx-maintenance --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 \
    multiplane rail 2 plane 6 off

Enable maintenance mode on plane 6 on all four rails without changing plane 2:
    $ nico-admin-cli spx-maintenance --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 \
    multiplane rail all plane 6 on

"
    )]
    Multiplane {
        #[arg(value_enum, value_name = "rail", help = "The literal keyword rail")]
        rail_keyword: RailKeyword,
        #[arg(value_name = "0-3|all", help = "Rail number (0-3), or all four rails")]
        rail: RailSelector,
        #[arg(value_enum, value_name = "plane", help = "The literal keyword plane")]
        plane_keyword: PlaneKeyword,
        #[arg(value_name = "0-7", value_parser = clap::value_parser!(u8).range(0..=7), help = "Plane number (0-7); planes 0-3 and 4-7 belong to separate SuperNICs")]
        plane: u8,
        #[arg(
            value_enum,
            value_name = "on|off",
            help = "maintenance mode on disables the plane; maintenance mode off enables it"
        )]
        mode: MaintenanceMode,
        #[arg(
            long,
            value_name = "REFERENCE",
            help = "Optional reference or reason for the change"
        )]
        reference: Option<String>,
    },
    #[command(
        about = "Show host maintenance configuration and each DPU's status",
        override_usage = "nico-admin-cli spx-maintenance --machine-id <MACHINE_ID> show",
        after_long_help = "\
EXAMPLES:

Show host requests and per-DPU maintenance state for a machine:
    $ nico-admin-cli spx-maintenance --machine-id fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80 show

"
    )]
    Show,
}

// Literal positional keywords keep the public command flat while checking its syntax.
#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum RailKeyword {
    Rail,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(super) enum PlaneKeyword {
    Plane,
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
            ["nico-admin-cli", "spx-maintenance"]
                .into_iter()
                .chain(arguments.split_whitespace()),
        )
        .map_err(|error| error.kind())?;
        match options.commands {
            Some(CliCommand::SpxMaintenance(args)) => Ok(args),
            _ => panic!("public CLI did not route to spx-maintenance"),
        }
    }

    #[test]
    fn public_command_preserves_selected_rail_plane_and_mode() {
        check_cases(
            [
                Case {
                    scenario: "first rail and plane, maintenance on",
                    input: "multiplane rail 0 plane 0 on",
                    expect: Yields((RailSelector::One(0), 0, MaintenanceMode::On, None)),
                },
                Case {
                    scenario: "last rail and plane, maintenance off with reference",
                    input: "multiplane rail 3 plane 7 off --reference SPX-42",
                    expect: Yields((
                        RailSelector::One(3),
                        7,
                        MaintenanceMode::Off,
                        Some("SPX-42".to_string()),
                    )),
                },
                Case {
                    scenario: "all rails, only specified plane",
                    input: "multiplane rail all plane 6 on",
                    expect: Yields((RailSelector::All, 6, MaintenanceMode::On, None)),
                },
            ],
            |selection| {
                let args = parse(&format!("--machine-id {MACHINE} {selection}"))?;
                assert_eq!(args.machine_id, MACHINE.parse::<MachineId>().unwrap());
                match args.action {
                    Action::Multiplane {
                        rail,
                        plane,
                        mode,
                        reference,
                        ..
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
                    input: format!("--machine-id {MACHINE} multiplane rail 4 plane 6 on"),
                    expect: Yields(ErrorKind::ValueValidation),
                },
                Case {
                    scenario: "plane outside 0-7",
                    input: format!("--machine-id {MACHINE} multiplane rail 2 plane 8 on"),
                    expect: Yields(ErrorKind::ValueValidation),
                },
                Case {
                    scenario: "invalid maintenance mode",
                    input: format!("--machine-id {MACHINE} multiplane rail 2 plane 6 true"),
                    expect: Yields(ErrorKind::InvalidValue),
                },
                Case {
                    scenario: "invalid rail keyword",
                    input: format!("--machine-id {MACHINE} multiplane rails 2 plane 6 on"),
                    expect: Yields(ErrorKind::InvalidValue),
                },
                Case {
                    scenario: "invalid plane keyword",
                    input: format!("--machine-id {MACHINE} multiplane rail 2 planes 6 on"),
                    expect: Yields(ErrorKind::InvalidValue),
                },
                Case {
                    scenario: "missing machine",
                    input: "multiplane rail 2 plane 6 on".to_string(),
                    expect: Yields(ErrorKind::MissingRequiredArgument),
                },
                Case {
                    scenario: "missing maintenance mode",
                    input: format!("--machine-id {MACHINE} multiplane rail 2 plane 6"),
                    expect: Yields(ErrorKind::MissingRequiredArgument),
                },
                Case {
                    scenario: "missing rail selector",
                    input: format!("--machine-id {MACHINE} multiplane rail plane 6 on"),
                    expect: Yields(ErrorKind::ValueValidation),
                },
                Case {
                    scenario: "missing plane selector",
                    input: format!("--machine-id {MACHINE} multiplane rail 2 plane"),
                    expect: Yields(ErrorKind::MissingRequiredArgument),
                },
                Case {
                    scenario: "invalid machine ID",
                    input: "--machine-id not-a-machine multiplane rail 2 plane 6 on".to_string(),
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
            "spx-maintenance",
            "--machine-id",
            MACHINE,
            "multiplane",
            "--help",
        ])
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::DisplayHelp);
        let help = error.to_string();
        assert!(
            help.contains(
                "--machine-id <MACHINE_ID> multiplane rail <0-3|all> plane <0-7> <on|off>"
            )
        );
        assert!(help.contains("EXAMPLES:"));
    }

    #[test]
    fn show_parses_for_selected_machine() {
        let args = parse(&format!("--machine-id {MACHINE} show")).unwrap();
        assert!(matches!(args.action, Action::Show));
        assert_eq!(args.machine_id, MACHINE.parse::<MachineId>().unwrap());
    }
}
