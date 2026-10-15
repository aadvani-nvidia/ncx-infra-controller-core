/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use std::convert::Infallible;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::{Bytes, Frame, Incoming};
use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper::{Request, Response, header};
use hyper_util::rt::{TokioExecutor, TokioIo};
use prost::Message;
use rpc::forge;
use tokio::net::TcpListener;
use tokio::process::Command;

#[tokio::test]
async fn spx_maintenance_public_commands_send_requests_and_show_requested_state() {
    let machine = "fm100psbtmb15tgh6q5duqb8ke5grng7ksd96hetbeie9nc5pvcca6eol80";
    let cases = [
        ("show", vec!["show"], None),
        (
            "all rails off",
            vec!["multiplane", "rail", "all", "plane", "6", "off"],
            Some((None, false)),
        ),
        (
            "single rail on",
            vec![
                "multiplane",
                "rail",
                "2",
                "plane",
                "6",
                "on",
                "--reference",
                "https://tickets.example.com/SPX-42",
            ],
            Some((Some(2), true)),
        ),
    ];
    for (scenario, arguments, expected) in cases {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api_url = format!("http://{}", listener.local_addr().unwrap());
        let called = Arc::new(Mutex::new(false));
        let server_called = called.clone();
        let configuration = forge::MachineSpxMultiplane {
            machine_id: Some(machine.parse().unwrap()),
            interfaces: vec![
                forge::MachineSpxMultiplaneInterface {
                    logical_ifname: "eth_r2_p6".to_string(),
                    rail: 2,
                    plane: 6,
                    maintenance_mode: false,
                    reference: Some("https://tickets.example.com/SPX-42".to_string()),
                    revision: "V2-T1791792000000000".to_string(),
                    applied_maintenance_mode: Some(false),
                    applied_revision: Some("V2-T1791792000000000".to_string()),
                    netdev: Some("A56p0".to_string()),
                    observed_at: Some(chrono::Utc::now().into()),
                    ..Default::default()
                },
                forge::MachineSpxMultiplaneInterface {
                    logical_ifname: "eth_r0_p0".to_string(),
                    rail: 0,
                    plane: 0,
                    maintenance_mode: true,
                    revision: "V1-T1791791900000000".to_string(),
                    ..Default::default()
                },
                forge::MachineSpxMultiplaneInterface {
                    logical_ifname: "eth_r1_p0".to_string(),
                    rail: 1,
                    maintenance_mode: true,
                    revision: "V1-T1791791900000000".to_string(),
                    error: Some("Xplane unavailable".to_string()),
                    observed_at: Some(chrono::Utc::now().into()),
                    ..Default::default()
                },
            ],
        };
        let server = tokio::spawn(async move {
            let (connection, _) = listener.accept().await.unwrap();
            http2::Builder::new(TokioExecutor::new()).serve_connection(TokioIo::new(connection), service_fn(move |request: Request<Incoming>| {
                let configuration = configuration.clone();
                let called = server_called.clone();
                async move {
                    let method = request.uri().path().to_string();
                    let payload = if method == "/forge.Forge/Version" { forge::BuildInfo::default().encode_to_vec() } else {
                        let body = request.into_body().collect().await.unwrap().to_bytes();
                        if let Some((rail, mode)) = expected {
                            assert_eq!(method, "/forge.Forge/UpdateMachineSpxMultiplaneMaintenance");
                            let request = forge::MachineSpxMultiplaneMaintenanceUpdateRequest::decode(body.slice(5..)).unwrap();
                            assert_eq!(request.machine_id, Some(machine.parse().unwrap()));
                            assert_eq!(request.plane, Some(6));
                            assert_eq!(request.maintenance_mode, Some(mode));
                            use forge::machine_spx_multiplane_maintenance_update_request::RailSelector;
                            assert_eq!(request.rail_selector, Some(rail.map(RailSelector::Rail).unwrap_or(RailSelector::AllRails(()))));
                            assert_eq!(request.reference.as_deref(), rail.map(|_| "https://tickets.example.com/SPX-42"));
                        } else {
                            assert_eq!(method, "/forge.Forge/GetMachineSpxMultiplane");
                            let request = forge::MachineSpxMultiplaneRequest::decode(body.slice(5..)).unwrap();
                            assert_eq!(request.machine_id, Some(machine.parse().unwrap()));
                        }
                        *called.lock().unwrap() = true;
                        configuration.encode_to_vec()
                    };
                    let mut data = vec![0];
                    data.extend_from_slice(&u32::try_from(payload.len()).unwrap().to_be_bytes());
                    data.extend_from_slice(&payload);
                    let mut trailers = hyper::HeaderMap::new();
                    trailers.insert("grpc-status", header::HeaderValue::from_static("0"));
                    let body = StreamBody::new(stream::iter([Ok::<_, Infallible>(Frame::data(Bytes::from(data))), Ok(Frame::trailers(trailers))]));
                    Ok::<_, Infallible>(Response::builder().header(header::CONTENT_TYPE, "application/grpc+tonic").body(body).unwrap())
                }
            })).await.unwrap();
        });
        let output = tokio::time::timeout(
            Duration::from_secs(10),
            Command::new(env!("CARGO_BIN_EXE_nico-admin-cli"))
                .env_clear()
                .args([
                    "--api-url",
                    &api_url,
                    "--root-ca-path",
                    "/unused/ca.crt",
                    "--client-cert-path",
                    "/unused/client.crt",
                    "--client-key-path",
                    "/unused/client.key",
                    "spx-maintenance",
                    "--machine-id",
                    machine,
                ])
                .args(arguments)
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await;
        server.abort();
        if let Err(error) = server.await {
            assert!(error.is_cancelled(), "{scenario}: server failed: {error}");
        }
        let output = output
            .expect("command completed within ten seconds")
            .unwrap();
        assert!(
            output.status.success(),
            "{scenario}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            *called.lock().unwrap(),
            "{scenario}: public command must call Core"
        );
        let text = String::from_utf8(output.stdout).unwrap();
        for value in [
            "Logical interface",
            "Requested maintenance mode",
            "Reference",
            "Revision",
            "Applied maintenance mode",
            "Status",
            "Netdev",
            "Applied",
            "Pending",
            "unknown",
            "A56p0",
            "eth_r2_p6",
            "eth_r0_p0",
            "off",
            "on",
            "https://tickets.example.com/SPX-42",
            "V2-T1791792000000000",
        ] {
            assert!(
                text.contains(value),
                "{scenario}: missing {value} in {text}"
            );
        }
        for (name, status) in [
            ("eth_r2_p6", "Applied"),
            ("eth_r0_p0", "Pending"),
            ("eth_r1_p0", "Error"),
        ] {
            let row = text.lines().find(|line| line.contains(name)).unwrap();
            assert!(
                row.contains(status),
                "{scenario}: incorrect status in {row}"
            );
        }
        assert!(text.contains("Xplane unavailable"));
    }
}
