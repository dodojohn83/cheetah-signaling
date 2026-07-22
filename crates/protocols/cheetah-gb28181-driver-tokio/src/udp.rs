//! UDP receive loop.

use crate::shared::{DriverAction, Shared};
use cheetah_gb28181_core::{GbAccessMachine, SipParser, encode_message};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio_util::sync::CancellationToken;
use tracing::{debug, trace, warn};

/// Runs a single UDP socket receive loop until cancelled or the socket fails.
///
/// The receive buffer is bounded to `max_datagram_size + 1` bytes so that
/// datagrams exceeding the configured maximum are detected and rejected rather
/// than silently truncated.
pub(crate) async fn run_udp<M>(
    shared: Arc<Shared<M>>,
    socket: Arc<UdpSocket>,
    cancel: CancellationToken,
) where
    M: GbAccessMachine + Send + 'static,
{
    let max = shared.max_datagram_size();
    let mut buf = vec![0u8; max.saturating_add(1)];
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                debug!("gb28181 udp receive loop cancelled");
                break;
            }
            result = socket.recv_from(&mut buf) => {
                match result {
                    Ok((len, source)) => {
                        if len > max {
                            warn!(%source, len, max, "rejecting oversized UDP datagram");
                            continue;
                        }
                        handle_datagram(&shared, &socket, &buf[..len], source).await;
                    }
                    Err(e) => {
                        warn!(error = %e, "gb28181 udp socket error; stopping receive loop");
                        break;
                    }
                }
            }
        }
    }
}

async fn handle_datagram<M>(
    shared: &Arc<Shared<M>>,
    socket: &UdpSocket,
    data: &[u8],
    source: SocketAddr,
) where
    M: GbAccessMachine + Send + 'static,
{
    let message = match SipParser::parse_datagram_with_profile(
        data,
        shared.parser_config(),
        shared.compatibility_profile(),
    ) {
        Ok(message) => message,
        Err(e) => {
            warn!(error = %e, %source, "failed to parse UDP SIP datagram");
            return;
        }
    };
    trace!(%source, "received UDP SIP datagram");

    let actions = match shared.handle_incoming(source, message, false) {
        Ok(actions) => actions,
        Err(e) => {
            warn!(error = %e, %source, "access machine rejected UDP datagram");
            return;
        }
    };

    for action in actions {
        match action {
            DriverAction::Send { message, target } => {
                let bytes = encode_message(&message);
                if let Err(e) = socket.send_to(&bytes, target).await {
                    warn!(error = %e, target = %target, "failed to send UDP SIP message");
                } else {
                    debug!(target = %target, "sent UDP SIP message");
                }
            }
            DriverAction::Emit(event) => shared.emit(event),
        }
    }
}
