//! Example out-of-process protocol plugin.
//!
//! This binary demonstrates the minimal surface of a Cheetah plugin runtime:
//!
//! - Listens on the address supplied by the host via `CHEETAH_PLUGIN_LISTEN_ADDRESS`.
//! - Generates a self-signed certificate whose SAN URI matches the plugin name.
//! - Exposes the `PluginRuntime` gRPC service implemented by
//!   [`cheetah_plugin_testkit::FakePluginRuntime`].
//! - Shuts down cleanly on `SIGINT`/`SIGTERM`.

use cheetah_plugin_testkit::{FakePluginRuntime, TestCerts};
use std::net::SocketAddr;
use tonic::transport::{Identity, ServerTlsConfig};

const DEFAULT_LISTEN_ADDRESS: &str = "127.0.0.1:0";
const DEFAULT_PLUGIN_NAME: &str = "examples/protocol-plugin";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let plugin_name =
        std::env::var("CHEETAH_PLUGIN_NAME").unwrap_or_else(|_| DEFAULT_PLUGIN_NAME.to_string());
    let listen_address = std::env::var("CHEETAH_PLUGIN_LISTEN_ADDRESS")
        .unwrap_or_else(|_| DEFAULT_LISTEN_ADDRESS.to_string());

    let addr: SocketAddr = listen_address.parse()?;

    let certs = TestCerts::generate(&plugin_name)?;
    let dir = tempfile::tempdir()?;
    let paths = certs.write_to_dir(dir.path())?;

    let identity = Identity::from_pem(
        std::fs::read_to_string(&paths.server_cert_path)?,
        std::fs::read_to_string(&paths.server_key_path)?,
    );
    let tls_config = ServerTlsConfig::new().identity(identity);

    let server = tokio::spawn(async move { FakePluginRuntime::serve(addr, tls_config).await });

    let handle = server.abort_handle();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        handle.abort();
    });

    match server.await {
        Ok(result) => result?,
        Err(join_err) if join_err.is_cancelled() => return Ok(()),
        Err(join_err) => return Err(join_err.into()),
    }

    Ok(())
}
