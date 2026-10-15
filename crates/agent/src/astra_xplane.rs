/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

//! Reconcile maintenance independently of attachment, HBN, and DHCP status.

use std::collections::HashMap;
use std::path::Path;

use eyre::WrapErr;
use futures_util::TryStreamExt;
use rpc::forge::{AstraConfig, AstraMultiplaneInterfaceStatus};
use rtnetlink::packet_route::link::{LinkAttribute, LinkMessage, Prop};
use tracing::Instrument;

use crate::xplane_client;

async fn interface_links() -> eyre::Result<Vec<LinkMessage>> {
    tracing::info!("Discovering Linux netdev names and altnames for Xplane maintenance");
    let (connection, handle, _receiver) = rtnetlink::new_connection()?;
    tokio::spawn(connection);
    let links: Vec<LinkMessage> = tokio::time::timeout(
        xplane_client::ATTEMPT_TIMEOUT,
        handle.link().get().execute().try_collect(),
    )
    .await
    .wrap_err("netdev discovery timed out")?
    .wrap_err("reading netdev names and altnames")?;
    tracing::info!(
        interface_count = links.len(),
        "Discovered Linux netdevs for Xplane maintenance"
    );
    Ok(links)
}

fn resolve_netdev(logical_ifname: &str, links: &[LinkMessage]) -> eyre::Result<String> {
    if !matches!(
        logical_ifname.as_bytes(),
        [
            b'e',
            b't',
            b'h',
            b'_',
            b'r',
            b'0'..=b'3',
            b'_',
            b'p',
            b'0'..=b'7'
        ]
    ) {
        return Err(eyre::eyre!(
            "invalid logical interface name {logical_ifname}"
        ));
    }
    let mut matched = None;
    for link in links {
        let Some(name) = link
            .attributes
            .iter()
            .find_map(|attribute| match attribute {
                LinkAttribute::IfName(name) => Some(name),
                _ => None,
            })
        else {
            continue;
        };
        let matches = name == logical_ifname || link.attributes.iter().any(|attribute| match attribute {
            LinkAttribute::PropList(properties) => properties.iter().any(|property| matches!(property, Prop::AltIfName(altname) if altname == logical_ifname)),
            _ => false,
        });
        if matches {
            if matched.is_some() {
                return Err(eyre::eyre!(
                    "multiple netdevs own logical interface {logical_ifname}"
                ));
            }
            matched = Some(name.clone());
        }
    }
    matched.ok_or_else(|| eyre::eyre!("no netdev owns logical interface {logical_ifname}"))
}

pub(crate) async fn reconcile(
    socket_path: &Path,
    config: Option<&AstraConfig>,
) -> Vec<AstraMultiplaneInterfaceStatus> {
    let Some(config) = config.filter(|c| {
        c.is_maintenance_config_present.unwrap_or(true) && !c.multiplane_interfaces.is_empty()
    }) else {
        tracing::info!(
            "Skipping Xplane maintenance reconciliation: no maintenance configuration or requested interfaces"
        );
        return Vec::new();
    };
    reconcile_with_links(socket_path, config, interface_links().await).await
}

async fn reconcile_with_links(
    socket_path: &Path,
    config: &AstraConfig,
    links: eyre::Result<Vec<LinkMessage>>,
) -> Vec<AstraMultiplaneInterfaceStatus> {
    tracing::info!(socket_path = %socket_path.display(), requested_interface_count = config.multiplane_interfaces.len(), discovered_interface_count = ?links.as_ref().map(Vec::len).ok(), "Validating Xplane maintenance mappings");
    // Validate all mappings before calling Xplane. Resolve through Linux
    // altnames and use the primary IfName as the PF name sent over gRPC.
    let mut statuses: Vec<_> = config
        .multiplane_interfaces
        .iter()
        .map(|request| {
            let mut status = AstraMultiplaneInterfaceStatus {
                logical_ifname: request.logical_ifname.clone(),
                revision: request.revision.clone(),
                ..Default::default()
            };
            let result = (|| {
                if config.multiplane_interfaces.len() > 32 {
                    return Err(eyre::eyre!(
                        "at most 32 multiplane interfaces are supported"
                    ));
                }
                if request.maintenance_mode.is_none() || request.revision.is_empty() {
                    return Err(eyre::eyre!("maintenance mode and revision are required"));
                }
                let links = links.as_ref().map_err(|error| eyre::eyre!("{error:#}"))?;
                resolve_netdev(&request.logical_ifname, links)
            })();
            match result {
                Ok(netdev) => status.netdev = Some(netdev),
                Err(error) => status.error = Some(format!("{error:#}")),
            }
            status
        })
        .collect();
    let mut owners = HashMap::<&str, usize>::new();
    for status in &statuses {
        if let Some(netdev) = status.netdev.as_deref() {
            *owners.entry(netdev).or_default() += 1;
        }
    }
    let duplicates: Vec<String> = owners
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(name, _)| name.to_string())
        .collect();
    for status in &mut statuses {
        if status
            .netdev
            .as_ref()
            .is_some_and(|name| duplicates.contains(name))
        {
            status.error =
                Some("multiple logical interfaces resolve to the same PF netdev".to_string());
        }
    }

    for (status, request) in statuses.iter().zip(&config.multiplane_interfaces) {
        tracing::info!(logical_ifname = %status.logical_ifname, netdev = ?status.netdev, revision = %status.revision, requested_maintenance_mode = ?request.maintenance_mode, error = ?status.error, "Xplane maintenance interface mapping validation result");
    }

    let client = if statuses.iter().any(|s| s.error.is_none()) {
        Some(xplane_client::connect(socket_path).await)
    } else {
        None
    };
    let statuses = futures::future::join_all(statuses.into_iter().zip(&config.multiplane_interfaces).map(|(mut status, request)| {
        let client = &client;
        let span = tracing::info_span!("xplane_maintenance_interface", logical_ifname = %status.logical_ifname, netdev = ?status.netdev, revision = %status.revision, requested_maintenance_mode = ?request.maintenance_mode);
        async move {
            tracing::info!("Starting Xplane maintenance interface reconciliation");
            if status.error.is_none() {
                match client.as_ref().expect("valid requests connect to Xplane") {
                    Err(error) => status.error = Some(format!("{error:#}")),
                    Ok(client) => {
                        let desired = request.maintenance_mode.expect("validated mode");
                        match xplane_client::reconcile(client.clone(), status.netdev.as_deref().expect("resolved PF"), desired).await {
                            Ok(actual) => {
                                status.maintenance_mode = Some(actual);
                                if actual != desired { status.error = Some("Xplane readback does not match requested maintenance mode".to_string()); }
                            },
                            Err(error) => status.error = Some(format!("{error:#}")),
                        }
                    },
                }
            }
            status.observed_at = Some(chrono::Utc::now().into());
            if let Some(error) = &status.error {
                tracing::warn!(logical_ifname = %status.logical_ifname, netdev = ?status.netdev, %error, "Xplane maintenance reconciliation failed; retrying on next config poll");
            }
            tracing::info!(observed_maintenance_mode = ?status.maintenance_mode, error = ?status.error, observed_at = ?status.observed_at, "Completed Xplane maintenance interface reconciliation");
            status
        }.instrument(span)
    })).await;
    tracing::info!(
        interface_count = statuses.len(),
        verified_interface_count = statuses.iter().filter(|s| s.error.is_none()).count(),
        failed_interface_count = statuses.iter().filter(|s| s.error.is_some()).count(),
        "Completed Xplane maintenance reconciliation batch"
    );
    statuses
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use rpc::forge::AstraMultiplaneInterface;

    use super::*;
    use crate::xplane_mock_server::{self, MockXplaneMaintenance};

    #[tokio::test]
    async fn absent_maintenance_config_skips_reconciliation() {
        for (scenario, config) in [
            ("no config", None),
            (
                "no interfaces",
                Some(AstraConfig {
                    is_maintenance_config_present: Some(true),
                    ..Default::default()
                }),
            ),
            (
                "config explicitly absent",
                Some(AstraConfig {
                    is_maintenance_config_present: Some(false),
                    multiplane_interfaces: vec![AstraMultiplaneInterface {
                        logical_ifname: "eth_r0_p0".to_string(),
                        maintenance_mode: Some(true),
                        revision: "V1-T1791792000000000".to_string(),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
            ),
        ] {
            assert!(
                reconcile(
                    Path::new("/unused-xplane-maintenance.sock"),
                    config.as_ref()
                )
                .await
                .is_empty(),
                "{scenario}"
            );
        }
    }

    fn link(name: &str, altnames: &[&str]) -> LinkMessage {
        let mut link = LinkMessage::default();
        link.attributes
            .push(LinkAttribute::IfName(name.to_string()));
        link.attributes.push(LinkAttribute::PropList(
            altnames
                .iter()
                .map(|name| Prop::AltIfName(name.to_string()))
                .collect(),
        ));
        link
    }

    #[test]
    fn logical_interface_resolves_to_primary_netdev() {
        for (scenario, name, links, expected) in [
            (
                "altname",
                "eth_r0_p0",
                vec![link("A56p0", &["eth_r0_p0"])],
                Some("A56p0"),
            ),
            (
                "primary name",
                "eth_r0_p0",
                vec![link("eth_r0_p0", &[])],
                Some("eth_r0_p0"),
            ),
            (
                "missing",
                "eth_r0_p1",
                vec![link("A56p0", &["eth_r0_p0"])],
                None,
            ),
            (
                "ambiguous",
                "eth_r0_p0",
                vec![link("A56p0", &["eth_r0_p0"]), link("A53p0", &["eth_r0_p0"])],
                None,
            ),
            (
                "invalid logical name",
                "eth_r0_p8",
                vec![link("A56p0", &["eth_r0_p8"])],
                None,
            ),
        ] {
            assert_eq!(
                resolve_netdev(name, &links).ok().as_deref(),
                expected,
                "{scenario}"
            );
        }
    }

    #[tokio::test]
    async fn xplane_reconciliation_reports_readback_retries_and_isolates_failures() {
        let directory = tempfile::tempdir().unwrap();
        let socket = directory.path().join("xplane.sock");
        let service = Arc::new(MockXplaneMaintenance::default());
        service
            .fail_set_for
            .lock()
            .await
            .insert("A56p1".to_string());
        service
            .readback_override
            .lock()
            .await
            .insert("A53p2".to_string(), false);
        service
            .stall_get_for
            .lock()
            .await
            .insert("A53p3".to_string());
        let server_path = socket.clone();
        let server_service = service.clone();
        let server = tokio::spawn(async move {
            xplane_mock_server::serve_with_service(&server_path, server_service).await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if xplane_client::connect(&socket).await.is_ok() {
                    break;
                }
                assert!(!server.is_finished(), "mock server stopped");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let request = |name: &str, mode| AstraMultiplaneInterface {
            logical_ifname: name.to_string(),
            maintenance_mode: mode,
            revision: "V1-T1791792000000000".to_string(),
            ..Default::default()
        };
        let mut config = AstraConfig {
            multiplane_interfaces: vec![
                request("eth_r0_p0", Some(false)),
                request("eth_r0_p1", Some(true)),
                request("eth_r0_p2", None),
                request("eth_r0_p3", Some(true)),
                request("eth_r1_p0", Some(true)),
                request("eth_r1_p1", Some(true)),
                request("eth_r1_p2", Some(true)),
                request("eth_r1_p3", Some(true)),
            ],
            ..Default::default()
        };
        let links = vec![
            link("A56p0", &["eth_r0_p0"]),
            link("A56p1", &["eth_r0_p1"]),
            link("A56p2", &["eth_r0_p2"]),
            link("A53p0", &["eth_r1_p0", "eth_r1_p1"]),
            link("A53p2", &["eth_r1_p2"]),
            link("A53p3", &["eth_r1_p3"]),
        ];
        let statuses = tokio::time::timeout(
            Duration::from_secs(5),
            reconcile_with_links(&socket, &config, Ok(links.clone())),
        )
        .await
        .unwrap();
        assert_eq!(statuses.len(), config.multiplane_interfaces.len());
        assert_eq!(statuses[0].netdev.as_deref(), Some("A56p0"));
        assert_eq!(statuses[0].maintenance_mode, Some(false));
        assert!(statuses[0].error.is_none());
        for status in &statuses[1..] {
            assert!(
                status.error.is_some(),
                "{} must report its failure",
                status.logical_ifname
            );
        }
        assert_eq!(
            statuses[6].maintenance_mode,
            Some(false),
            "report actual state when readback disagrees"
        );
        assert!(
            statuses[7].maintenance_mode.is_none(),
            "timeout cannot claim application"
        );
        let mut calls = service.set_calls.lock().await.clone();
        calls.sort();
        assert_eq!(
            calls,
            vec![
                ("A53p2".to_string(), true),
                ("A56p0".to_string(), false),
                ("A56p1".to_string(), true)
            ]
        );

        // Same desired revision retries a failed port, while verified ports
        // avoid redundant sets. No local success cache hides server drift.
        service.fail_set_for.lock().await.clear();
        config.multiplane_interfaces.truncate(2);
        let retry = reconcile_with_links(&socket, &config, Ok(links)).await;
        assert!(retry.iter().all(|s| s.error.is_none()));
        assert_eq!(retry[1].maintenance_mode, Some(true));
        let calls = service.set_calls.lock().await;
        assert_eq!(calls.iter().filter(|(name, _)| name == "A56p0").count(), 1);
        assert_eq!(calls.iter().filter(|(name, _)| name == "A56p1").count(), 2);
        drop(calls);

        let missing_socket = directory.path().join("missing.sock");
        let failures = reconcile_with_links(
            &missing_socket,
            &config,
            Ok(vec![
                link("A56p0", &["eth_r0_p0"]),
                link("A56p1", &["eth_r0_p1"]),
            ]),
        )
        .await;
        assert!(
            failures
                .iter()
                .all(|s| s.error.is_some() && s.maintenance_mode.is_none())
        );
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
    }
}
