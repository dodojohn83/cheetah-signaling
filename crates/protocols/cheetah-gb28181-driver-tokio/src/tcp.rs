//! TCP listener and per-connection framing loop.

use crate::shared::{Shared, SourceSlot};
use cheetah_gb28181_core::{AccessOutput, GbAccessMachine, SipParser, encode_message};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use tracing::{debug, trace, warn};

/// Accepts TCP connections on a single listener, enforcing the global and
/// per-source connection limits, until cancelled.
///
/// Each accepted connection runs in its own task holding an owned connection
/// permit and a per-source slot; both are released when the task ends, which is
/// also how shutdown drain observes that all connections have closed.
pub(crate) async fn run_tcp_listener<M>(
    shared: Arc<Shared<M>>,
    listener: TcpListener,
    cancel: CancellationToken,
) where
    M: GbAccessMachine + Send + 'static,
{
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                debug!("gb28181 tcp accept loop cancelled");
                break;
            }
            result = listener.accept() => {
                let (stream, peer) = match result {
                    Ok(pair) => pair,
                    Err(e) => {
                        warn!(error = %e, "gb28181 tcp accept failed");
                        continue;
                    }
                };

                let permit = match shared.conn_permits().try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        warn!(%peer, "gb28181 tcp connection limit reached; rejecting");
                        drop(stream);
                        continue;
                    }
                };

                let Some(slot) = shared.acquire_source(peer.ip()) else {
                    warn!(%peer, "gb28181 per-source tcp connection limit reached; rejecting");
                    drop(stream);
                    drop(permit);
                    continue;
                };

                let conn_shared = shared.clone();
                let conn_cancel = cancel.clone();
                tokio::spawn(async move {
                    let _permit: OwnedSemaphorePermit = permit;
                    let _slot: SourceSlot<M> = slot;
                    handle_connection(&conn_shared, stream, peer, conn_cancel).await;
                });
            }
        }
    }
}

/// Reads and frames SIP messages from a single TCP connection.
async fn handle_connection<M>(
    shared: &Arc<Shared<M>>,
    mut stream: TcpStream,
    peer: SocketAddr,
    cancel: CancellationToken,
) where
    M: GbAccessMachine + Send + 'static,
{
    let mut parser = SipParser::new(shared.parser_config());
    let mut chunk = vec![0u8; shared.tcp_read_chunk_bytes().max(1)];
    let idle = shared.tcp_idle_timeout();

    loop {
        let read = tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                debug!(%peer, "gb28181 tcp connection cancelled");
                break;
            }
            read = timeout(idle, stream.read(&mut chunk)) => read,
        };

        let n = match read {
            Err(_elapsed) => {
                debug!(%peer, "gb28181 tcp connection idle timeout; closing");
                break;
            }
            Ok(Ok(0)) => {
                trace!(%peer, "gb28181 tcp connection closed by peer");
                break;
            }
            Ok(Ok(n)) => n,
            Ok(Err(e)) => {
                warn!(error = %e, %peer, "gb28181 tcp read error");
                break;
            }
        };

        if let Err(e) = parser.feed(&chunk[..n]) {
            warn!(error = %e, %peer, "gb28181 tcp stream exceeded parser buffer; closing");
            break;
        }

        if !drain_messages(shared, &mut parser, &mut stream, peer).await {
            break;
        }
    }

    let _ = stream.shutdown().await;
}

/// Pops all currently framed messages, dispatching outputs. Returns `false`
/// when the connection must be closed (framing error or write failure).
async fn drain_messages<M>(
    shared: &Arc<Shared<M>>,
    parser: &mut SipParser,
    stream: &mut TcpStream,
    peer: SocketAddr,
) -> bool
where
    M: GbAccessMachine + Send + 'static,
{
    loop {
        match parser.pop_message() {
            None => return true,
            Some(Err(e)) => {
                warn!(error = %e, %peer, "gb28181 tcp framing error; closing connection");
                return false;
            }
            Some(Ok(message)) => {
                trace!(%peer, "received TCP SIP message");
                let outputs = match shared.process_message(peer, message) {
                    Ok(outputs) => outputs,
                    Err(e) => {
                        warn!(error = %e, %peer, "access machine rejected TCP message");
                        return false;
                    }
                };
                for output in outputs {
                    match output {
                        AccessOutput::SendResponse(response) => {
                            let bytes = encode_message(&response);
                            if let Err(e) = stream.write_all(&bytes).await {
                                warn!(error = %e, %peer, "failed to write TCP SIP response");
                                return false;
                            }
                            debug!(%peer, "sent TCP SIP response");
                        }
                        AccessOutput::EmitEvent(event) => shared.emit(event),
                    }
                }
            }
        }
    }
}
