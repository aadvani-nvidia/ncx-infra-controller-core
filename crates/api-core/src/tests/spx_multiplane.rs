/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use carbide_uuid::machine::MachineId;
use model::expected_machine::ExpectedInterface;
use rpc::forge;
use rpc::forge::forge_server::Forge;
use tonic::Request;

use crate::cfg::file::EwEthersConfig;
use crate::tests::common::api_fixtures::{self, TestEnv, TestEnvOverrides, TestManagedHost};

async fn fixture(pool: sqlx::PgPool) -> (TestEnv, TestManagedHost) {
    let mut config = api_fixtures::get_config();
    config.ewethers_config = Some(EwEthersConfig {
        enabled: true,
        astra_enabled: true,
        ..Default::default()
    });
    let env = api_fixtures::create_test_env_with_overrides(
        pool,
        TestEnvOverrides {
            config: Some(config),
            ..Default::default()
        },
    )
    .await;
    let host = api_fixtures::create_managed_host(&env).await;
    let mut txn = env.db_txn().await;
    let machine = host.host().db_machine(&mut txn).await;
    let mut expected = db::expected_machine::find_by_bmc_mac_address(
        &mut *txn,
        machine.status.bmc_info.mac.unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    for rail in 0..4 {
        for plane in 0..8 {
            expected.data.interfaces.push(ExpectedInterface {
                logical_ifname: Some(format!("eth_r{rail}_p{plane}")),
                cerebro_ifname: Some(
                    model::machine::astra::ASTRA_NIC_MAPPINGS[(rail * 8 + plane) as usize]
                        .cerebro_ifname
                        .to_string(),
                ),
                nic_type: Some("CX9".to_string()),
                ..Default::default()
            });
        }
    }
    db::expected_machine::update(&mut txn, &expected)
        .await
        .unwrap();
    txn.commit().await.unwrap();
    (env, host)
}

fn update_request(
    id: MachineId,
    rail: Option<u32>,
    plane: u32,
    mode: bool,
    reference: Option<&str>,
) -> forge::MachineSpxMultiplaneMaintenanceUpdateRequest {
    use forge::machine_spx_multiplane_maintenance_update_request::RailSelector;
    forge::MachineSpxMultiplaneMaintenanceUpdateRequest {
        machine_id: Some(id),
        rail_selector: Some(
            rail.map(RailSelector::Rail)
                .unwrap_or(RailSelector::AllRails(())),
        ),
        plane: Some(plane),
        maintenance_mode: Some(mode),
        reference: reference.map(str::to_string),
    }
}

async fn persisted(env: &TestEnv, id: MachineId) -> serde_json::Value {
    sqlx::query_scalar("SELECT spx_multiplane FROM machines WHERE id=$1")
        .bind(id)
        .fetch_one(&env.pool)
        .await
        .unwrap()
}

#[crate::sqlx_test]
async fn spx_multiplane_readback_persists_without_attachments_and_rejects_stale_status(
    pool: sqlx::PgPool,
) {
    let (env, host) = fixture(pool).await;
    let id: MachineId = host.id.into();
    let requested = env
        .api
        .update_machine_spx_multiplane_maintenance(Request::new(update_request(
            id,
            Some(0),
            0,
            false,
            None,
        )))
        .await
        .unwrap()
        .into_inner();
    let revision = requested.interfaces[0].revision.clone();
    let observed_at = chrono::Utc::now();
    let read = || {
        env.api
            .get_machine_spx_multiplane(Request::new(forge::MachineSpxMultiplaneRequest {
                machine_id: Some(id),
            }))
    };
    let report = |mode, revision: String, offset, is_maintenance_status_present| {
        env.api
            .record_dpu_network_status(Request::new(forge::DpuNetworkStatus {
                dpu_machine_id: Some(host.dpu().id),
                dpu_health: Some(rpc::health::HealthReport {
                    source: "forge-dpu-agent".to_string(),
                    ..Default::default()
                }),
                astra_config_status: Some(forge::AstraConfigStatus {
                    is_attachment_status_present: Some(false),
                    is_maintenance_status_present,
                    multiplane_interfaces_status: vec![forge::AstraMultiplaneInterfaceStatus {
                        logical_ifname: "eth_r0_p0".to_string(),
                        revision,
                        maintenance_mode: Some(mode),
                        netdev: Some("A56p0".to_string()),
                        observed_at: Some((observed_at + chrono::Duration::seconds(offset)).into()),
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                ..Default::default()
            }))
    };
    report(false, revision.clone(), 0, Some(true))
        .await
        .unwrap();
    let applied = read().await.unwrap().into_inner();
    let port = &applied.interfaces[0];
    assert_eq!(port.applied_maintenance_mode, Some(false));
    assert_eq!(port.applied_revision.as_deref(), Some(revision.as_str()));
    assert_eq!(port.netdev.as_deref(), Some("A56p0"));
    assert!(port.error.is_none());
    assert_eq!(
        port.revision, revision,
        "observations do not bump desired revisions"
    );
    let stored = persisted(&env, id).await;
    assert_eq!(
        stored["interfaces"]["eth_r0_p0"]["observation"]["maintenance_mode"],
        false
    );

    // A mismatching readback cannot be acknowledged as applied, even if the
    // agent accidentally omitted its error. A later good report recovers.
    // An explicit absent status is ignored, even if entries were supplied.
    // Omission preserves compatibility with agents predating the presence flag.
    for (presence, expected_mode) in [(Some(false), false), (None, true)] {
        report(true, revision.clone(), 1, presence).await.unwrap();
        let result = read().await.unwrap().into_inner();
        assert_eq!(
            result.interfaces[0].applied_maintenance_mode,
            Some(expected_mode)
        );
        assert_eq!(result.interfaces[0].error.is_some(), expected_mode);
        assert_eq!(
            result.interfaces[0].applied_revision.is_none(),
            expected_mode
        );
    }
    report(false, revision.clone(), 2, Some(true))
        .await
        .unwrap();
    report(true, revision.clone(), 1, Some(true)).await.unwrap();
    let recovered = read().await.unwrap().into_inner();
    assert!(recovered.interfaces[0].error.is_none());
    assert_eq!(
        recovered.interfaces[0].applied_maintenance_mode,
        Some(false)
    );

    let changed = env
        .api
        .update_machine_spx_multiplane_maintenance(Request::new(update_request(
            id,
            Some(0),
            0,
            true,
            None,
        )))
        .await
        .unwrap()
        .into_inner();
    assert_ne!(changed.interfaces[0].revision, revision);
    assert_eq!(
        changed.interfaces[0].applied_revision.as_deref(),
        Some(revision.as_str()),
        "new request remains pending with previous readback"
    );
    report(true, revision, 3, Some(true)).await.unwrap();
    let pending = read().await.unwrap().into_inner();
    assert_eq!(
        pending.interfaces[0].applied_maintenance_mode,
        Some(false),
        "delayed old revision is ignored"
    );
    assert_ne!(
        pending.interfaces[0].applied_revision.as_ref(),
        Some(&pending.interfaces[0].revision)
    );
    // A newer desired revision takes precedence even if the agent clock moved
    // backwards; timestamp ordering only rejects replays of the same revision.
    report(true, pending.interfaces[0].revision.clone(), 0, Some(true))
        .await
        .unwrap();
    let current = read().await.unwrap().into_inner();
    assert_eq!(current.interfaces[0].applied_maintenance_mode, Some(true));
    assert_eq!(
        current.interfaces[0].applied_revision.as_ref(),
        Some(&current.interfaces[0].revision)
    );

    // NIC discovery may start after maintenance delivery. A maintenance-only
    // report must preserve Weave's attachment observations once NICs exist.
    let mut txn = env.db_txn().await;
    let mac = "00:11:22:33:44:55".parse().unwrap();
    db::dpa_interface::persist(
        model::dpa_interface::NewDpaInterface {
            machine_id: *host.id,
            mac_address: mac,
            device_type: "ConnectX9".to_string(),
            pci_name: "0005:06:00.0".to_string(),
            device_description: None,
            interface_type: model::dpa_interface::DpaInterfaceType::Astra,
        },
        &mut txn,
    )
    .await
    .unwrap();
    let attachments = model::machine::spx::MachineSpxStatusObservation {
        spx_attachments: vec![model::machine::spx::MachineSpxAttachmentStatusObservation {
            mac_address: mac,
            ..Default::default()
        }],
        ..Default::default()
    };
    assert!(matches!(
        db::machine::update_spx_status_observation(&mut txn, &host.id, &attachments)
            .await
            .unwrap(),
        db::ConditionalWrite::Applied(())
    ));
    txn.commit().await.unwrap();
    report(true, current.interfaces[0].revision.clone(), 4, Some(true))
        .await
        .unwrap();
    let stored_attachments: serde_json::Value =
        sqlx::query_scalar("SELECT spx_status_observation FROM machines WHERE id=$1")
            .bind(id)
            .fetch_one(&env.pool)
            .await
            .unwrap();
    assert_eq!(
        stored_attachments,
        serde_json::to_value(&attachments).unwrap()
    );

    // Both an explicit present flag and omission by older agents retain full
    // attachment-report semantics: an observed empty list clears attachments.
    for presence in [Some(true), None] {
        let mut txn = env.db_txn().await;
        assert!(matches!(
            db::machine::update_spx_status_observation(&mut txn, &host.id, &attachments)
                .await
                .unwrap(),
            db::ConditionalWrite::Applied(())
        ));
        txn.commit().await.unwrap();
        env.api
            .record_dpu_network_status(Request::new(forge::DpuNetworkStatus {
                dpu_machine_id: Some(host.dpu().id),
                dpu_health: Some(rpc::health::HealthReport {
                    source: "forge-dpu-agent".to_string(),
                    ..Default::default()
                }),
                astra_config_status: Some(forge::AstraConfigStatus {
                    is_attachment_status_present: presence,
                    is_maintenance_status_present: Some(false),
                    ..Default::default()
                }),
                ..Default::default()
            }))
            .await
            .unwrap();
        let stored_attachments: serde_json::Value =
            sqlx::query_scalar("SELECT spx_status_observation FROM machines WHERE id=$1")
                .bind(id)
                .fetch_one(&env.pool)
                .await
                .unwrap();
        assert_eq!(stored_attachments["spx_attachments"], serde_json::json!([]));
    }
}

#[crate::sqlx_test]
async fn spx_multiplane_requests_persist_and_reach_dpu_without_attachments(pool: sqlx::PgPool) {
    let (env, host) = fixture(pool).await;
    let id: MachineId = host.id.into();
    let poll = || {
        env.api.get_managed_host_network_config(Request::new(
            forge::ManagedHostNetworkConfigRequest {
                dpu_machine_id: Some(host.dpu().id),
            },
        ))
    };
    let initial = poll()
        .await
        .unwrap()
        .into_inner()
        .astra_config
        .expect("maintenance must be delivered before NIC discovery");
    assert!(initial.astra_attachments.is_empty());
    assert_eq!(initial.is_attachment_config_present, Some(false));
    assert_eq!(initial.is_maintenance_config_present, Some(true));
    assert_eq!(initial.multiplane_interfaces.len(), 32);
    assert!(
        initial
            .multiplane_interfaces
            .iter()
            .all(|entry| entry.maintenance_mode == Some(true))
    );
    assert_eq!(
        persisted(&env, id).await["interfaces"]
            .as_object()
            .unwrap()
            .len(),
        32
    );

    let request = update_request(
        id,
        Some(2),
        6,
        false,
        Some("https://tickets.example.com/SPX-42"),
    );
    let updated = env
        .api
        .update_machine_spx_multiplane_maintenance(Request::new(request.clone()))
        .await
        .unwrap()
        .into_inner();
    let entry = updated
        .interfaces
        .iter()
        .find(|entry| entry.logical_ifname == "eth_r2_p6")
        .unwrap();
    assert!(!entry.maintenance_mode);
    assert!(entry.applied_maintenance_mode.is_none());
    assert!(entry.applied_revision.is_none());
    assert!(entry.netdev.is_none());
    let revision = entry.revision.clone();
    let delivered = poll().await.unwrap().into_inner().astra_config.unwrap();
    let entry = delivered
        .multiplane_interfaces
        .iter()
        .find(|entry| entry.logical_ifname == "eth_r2_p6")
        .unwrap();
    assert_eq!(entry.maintenance_mode, Some(false));
    assert_eq!(entry.revision, revision);
    assert_eq!(
        entry.reference.as_deref(),
        Some("https://tickets.example.com/SPX-42")
    );
    assert_eq!(
        persisted(&env, id).await["interfaces"]["eth_r2_p6"]["maintenance_mode"],
        false
    );

    // Repeated requests preserve the revision, and omission preserves the URL.
    let preserved = env
        .api
        .update_machine_spx_multiplane_maintenance(Request::new(update_request(
            id,
            Some(2),
            6,
            false,
            None,
        )))
        .await
        .unwrap()
        .into_inner();
    let entry = preserved
        .interfaces
        .iter()
        .find(|entry| entry.logical_ifname == "eth_r2_p6")
        .unwrap();
    assert_eq!(entry.revision, revision);
    assert_eq!(
        entry.reference.as_deref(),
        Some("https://tickets.example.com/SPX-42")
    );
    let cleared = env
        .api
        .update_machine_spx_multiplane_maintenance(Request::new(update_request(
            id,
            Some(2),
            6,
            false,
            Some(""),
        )))
        .await
        .unwrap()
        .into_inner();
    let entry = cleared
        .interfaces
        .iter()
        .find(|entry| entry.logical_ifname == "eth_r2_p6")
        .unwrap();
    assert!(entry.reference.is_none());
    assert_ne!(entry.revision, revision);

    let all = env
        .api
        .update_machine_spx_multiplane_maintenance(Request::new(update_request(
            id, None, 1, false, None,
        )))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        all.interfaces
            .iter()
            .filter(|entry| entry.plane == 1 && !entry.maintenance_mode)
            .count(),
        4
    );
    assert!(
        all.interfaces
            .iter()
            .filter(|entry| entry.plane == 5)
            .all(|entry| entry.maintenance_mode)
    );

    // Removing one declaration makes rail-all reject without any partial writes.
    let before = persisted(&env, id).await;
    let mut txn = env.db_txn().await;
    let machine = host.host().db_machine(&mut txn).await;
    let mut expected = db::expected_machine::find_by_bmc_mac_address(
        &mut *txn,
        machine.status.bmc_info.mac.unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    expected
        .data
        .interfaces
        .retain(|entry| entry.logical_ifname.as_deref() != Some("eth_r3_p7"));
    db::expected_machine::update(&mut txn, &expected)
        .await
        .unwrap();
    txn.commit().await.unwrap();
    let error = env
        .api
        .update_machine_spx_multiplane_maintenance(Request::new(update_request(
            id, None, 7, false, None,
        )))
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    assert_eq!(persisted(&env, id).await, before);

    let cases = [
        (
            "out of range plane",
            forge::MachineSpxMultiplaneMaintenanceUpdateRequest {
                plane: Some(8),
                ..request.clone()
            },
        ),
        (
            "out of range rail",
            update_request(id, Some(4), 0, false, None),
        ),
        (
            "missing maintenance mode",
            forge::MachineSpxMultiplaneMaintenanceUpdateRequest {
                maintenance_mode: None,
                ..request.clone()
            },
        ),
        (
            "missing rail",
            forge::MachineSpxMultiplaneMaintenanceUpdateRequest {
                rail_selector: None,
                ..request
            },
        ),
    ];
    for (scenario, request) in cases {
        let error = env
            .api
            .update_machine_spx_multiplane_maintenance(Request::new(request))
            .await
            .unwrap_err();
        assert_eq!(error.code(), tonic::Code::InvalidArgument, "{scenario}");
        assert_eq!(persisted(&env, id).await, before, "{scenario}");
    }
}

#[crate::sqlx_test]
async fn spx_multiplane_concurrent_updates_preserve_other_rails(pool: sqlx::PgPool) {
    let (env, host) = fixture(pool).await;
    let id: MachineId = host.id.into();
    env.api
        .get_machine_spx_multiplane(Request::new(forge::MachineSpxMultiplaneRequest {
            machine_id: Some(id),
        }))
        .await
        .unwrap();
    let mut blocker = env.db_txn().await;
    db::machine::lock_spx_multiplane(&mut blocker, &host.id.into())
        .await
        .unwrap();
    let first_api = env.api.clone();
    let second_api = env.api.clone();
    let first = tokio::spawn(async move {
        first_api
            .update_machine_spx_multiplane_maintenance(Request::new(update_request(
                id,
                Some(0),
                0,
                false,
                None,
            )))
            .await
    });
    let second = tokio::spawn(async move {
        second_api
            .update_machine_spx_multiplane_maintenance(Request::new(update_request(
                id,
                Some(1),
                0,
                false,
                None,
            )))
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE datname=current_database() AND wait_event_type='Lock' AND query LIKE 'SELECT spx_multiplane FROM machines%'")
                .fetch_one(&env.pool).await.unwrap();
            if waiting == 2 { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("both writers contend for the host lock");
    blocker.commit().await.unwrap();
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    let saved = persisted(&env, id).await;
    assert_eq!(saved["interfaces"]["eth_r0_p0"]["maintenance_mode"], false);
    assert_eq!(saved["interfaces"]["eth_r1_p0"]["maintenance_mode"], false);
    assert_eq!(saved["interfaces"]["eth_r2_p0"]["maintenance_mode"], true);
}
