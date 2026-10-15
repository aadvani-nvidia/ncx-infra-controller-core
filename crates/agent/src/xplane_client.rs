/*
 * SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0
 */

use std::path::Path;
use std::time::Duration;

use eyre::WrapErr;
use hyper_util::rt::TokioIo;
use tokio::net::UnixStream;
use tonic::transport::{Channel, Endpoint};
use tower::service_fn;

use crate::xplane_proto::xplane_maintenance_client::XplaneMaintenanceClient;
use crate::xplane_proto::{GetLinkMaintenanceRequest, SetLinkMaintenanceRequest};

// Local unary requests and connection establishment each have a three-second
// deadline, including channel waits. Failed attempts retry on the next config
// poll. Ports share one channel and run concurrently, avoiding 32 serial waits.
pub(crate) const ATTEMPT_TIMEOUT: Duration = Duration::from_secs(3);

pub(crate) async fn connect(socket_path: &Path) -> eyre::Result<XplaneMaintenanceClient<Channel>> {
    tracing::info!(socket_path = %socket_path.display(), timeout_seconds = ATTEMPT_TIMEOUT.as_secs(), "Connecting to Xplane maintenance service");
    let path = socket_path.to_owned();
    let channel = tokio::time::timeout(
        ATTEMPT_TIMEOUT,
        Endpoint::from_static("http://localhost").connect_with_connector(service_fn(move |_| {
            let path = path.clone();
            async move { UnixStream::connect(path).await.map(TokioIo::new) }
        })),
    )
    .await
    .inspect_err(|error| tracing::info!(socket_path = %socket_path.display(), %error, "Xplane maintenance connection timed out"))
    .wrap_err("xplane connection timed out")?
    .inspect_err(|error| tracing::info!(socket_path = %socket_path.display(), %error, "Xplane maintenance connection failed"))
    .wrap_err_with(|| format!("connecting to xplane at {}", socket_path.display()))?;
    tracing::info!(socket_path = %socket_path.display(), "Connected to Xplane maintenance service");
    Ok(XplaneMaintenanceClient::new(channel))
}

async fn read(
    client: &mut XplaneMaintenanceClient<Channel>,
    netdev: &str,
) -> eyre::Result<Result<bool, tonic::Status>> {
    tracing::info!(
        pf_name = netdev,
        timeout_seconds = ATTEMPT_TIMEOUT.as_secs(),
        "Calling Xplane GetLinkMaintenance"
    );
    let mut request = tonic::Request::new(GetLinkMaintenanceRequest {
        pf_name: netdev.to_string(),
    });
    request.set_timeout(ATTEMPT_TIMEOUT);
    let result = tokio::time::timeout(ATTEMPT_TIMEOUT, client.get_link_maintenance(request))
        .await
        .inspect_err(
            |error| tracing::info!(pf_name = netdev, %error, "Xplane GetLinkMaintenance timed out"),
        )
        .wrap_err("xplane readback timed out")?
        .map(|response| response.into_inner().enabled);
    match &result {
        Ok(enabled) => tracing::info!(
            pf_name = netdev,
            maintenance_mode = *enabled,
            "Received Xplane GetLinkMaintenance response"
        ),
        Err(error) => {
            tracing::info!(pf_name = netdev, grpc_status_code = ?error.code(), %error, "Xplane GetLinkMaintenance failed")
        }
    }
    Ok(result)
}

/// Read every poll to detect drift or an Xplane restart. Set only when readback
/// differs or is not yet initialized, and verify again after setting.
pub(crate) async fn reconcile(
    mut client: XplaneMaintenanceClient<Channel>,
    netdev: &str,
    maintenance_mode: bool,
) -> eyre::Result<bool> {
    match read(&mut client, netdev).await? {
        Ok(actual) if actual == maintenance_mode => {
            tracing::info!(
                pf_name = netdev,
                maintenance_mode,
                "Xplane maintenance already matches; skipping SetLinkMaintenance"
            );
            return Ok(actual);
        }
        Ok(actual) => {
            tracing::info!(
                pf_name = netdev,
                requested_maintenance_mode = maintenance_mode,
                observed_maintenance_mode = actual,
                "Xplane maintenance differs from requested state"
            );
        }
        Err(error) if error.code() == tonic::Code::NotFound => {
            tracing::info!(
                pf_name = netdev,
                maintenance_mode,
                "Xplane maintenance state is not initialized; setting requested state"
            );
        }
        Err(error) => return Err(error).wrap_err("reading xplane maintenance mode"),
    }
    let mut request = tonic::Request::new(SetLinkMaintenanceRequest {
        pf_name: netdev.to_string(),
        enabled: Some(maintenance_mode),
    });
    request.set_timeout(ATTEMPT_TIMEOUT);
    tracing::info!(
        pf_name = netdev,
        maintenance_mode,
        timeout_seconds = ATTEMPT_TIMEOUT.as_secs(),
        "Calling Xplane SetLinkMaintenance"
    );
    tokio::time::timeout(ATTEMPT_TIMEOUT, client.set_link_maintenance(request))
        .await
        .inspect_err(|error| tracing::info!(pf_name = netdev, maintenance_mode, %error, "Xplane SetLinkMaintenance timed out"))
        .wrap_err("xplane maintenance update timed out")?
        .inspect_err(|error| tracing::info!(pf_name = netdev, maintenance_mode, grpc_status_code = ?error.code(), %error, "Xplane SetLinkMaintenance failed"))
        .wrap_err("setting xplane maintenance mode")?;
    tracing::info!(
        pf_name = netdev,
        maintenance_mode,
        "Xplane SetLinkMaintenance accepted; verifying readback"
    );
    read(&mut client, netdev)
        .await?
        .wrap_err("verifying xplane maintenance mode")
}
