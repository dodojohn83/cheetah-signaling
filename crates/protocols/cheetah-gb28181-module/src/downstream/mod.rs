//! Sans-I/O GB28181 lower-platform (downstream) access state machine.
//!
//! The server acts as an upper platform: it accepts `REGISTER` and `MESSAGE`
//! requests from lower GB28181 platforms, authenticates them, and emits domain
//! events for catalog/alarm/position/state payloads. It can also issue outbound
//! `MESSAGE` queries such as `Catalog` requests to a registered lower platform.

mod config;
mod error;
mod link;
mod message;
mod register;

#[cfg(test)]
mod tests;

pub use config::DownstreamConfig;
pub use error::DownstreamError;

use crate::downstream::link::LinkTable;
use crate::events::{DevicePresence, Gb28181Event};
use crate::ports::CredentialProvider;
use crate::types::DeviceId;
use cheetah_gb28181_core::{DigestContext, DigestQop, DigestReplayCache, Method, SipMessage};
use secrecy::ExposeSecret;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};

/// An input to the downstream platform handler.
///
/// The `Sip` variant is intentionally larger than `Command` because it carries
/// the parsed wire message; boxing would only add an allocation without
/// reducing the amount of data that must be passed through the state machine.
#[derive(Clone)]
#[allow(clippy::large_enum_variant)]
pub enum DownstreamInput {
    /// A SIP message received from the network.
    Sip {
        /// Source address of the message.
        source: SocketAddr,
        /// Monotonic second counter.
        now: u64,
        /// Parsed SIP message.
        message: SipMessage,
    },
    /// An application command to send to a registered lower platform.
    Command {
        /// Monotonic second counter.
        now: u64,
        /// Command to execute.
        command: DownstreamCommand,
    },
}

impl std::fmt::Debug for DownstreamInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownstreamInput::Sip { source, now, .. } => f
                .debug_struct("Sip")
                .field("source", source)
                .field("now", now)
                .field("message", &"[REDACTED]")
                .finish(),
            DownstreamInput::Command { now, command } => f
                .debug_struct("Command")
                .field("now", now)
                .field("command", command)
                .finish(),
        }
    }
}

/// A command sent to a registered lower platform.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DownstreamCommand {
    /// Query the lower platform for its catalog.
    QueryCatalog {
        /// Lower-platform identifier.
        platform_id: DeviceId,
        /// Sequence number for the query.
        sn: String,
    },
}

/// An output from the downstream platform handler.
#[derive(Clone)]
pub enum DownstreamOutput {
    /// Send a SIP response to the transport.
    SendResponse(SipMessage),
    /// Send a SIP request to the given destination.
    SendRequest(SipMessage, SocketAddr),
    /// Emit a domain event for downstream consumers.
    EmitEvent(Gb28181Event),
}

impl std::fmt::Debug for DownstreamOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownstreamOutput::SendResponse(_) | DownstreamOutput::SendRequest(_, _) => {
                f.debug_tuple("SendSip").field(&"[REDACTED]").finish()
            }
            DownstreamOutput::EmitEvent(event) => f.debug_tuple("EmitEvent").field(event).finish(),
        }
    }
}

/// Sans-I/O state machine for lower-platform GB28181 access.
pub struct Gb28181Downstream<P: CredentialProvider> {
    config: DownstreamConfig,
    digest_context: DigestContext,
    replay_cache: DigestReplayCache,
    credential_provider: P,
    tag_counter: AtomicU64,
    links: LinkTable,
}

impl<P: CredentialProvider> std::fmt::Debug for Gb28181Downstream<P> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Gb28181Downstream")
            .field("config", &self.config)
            .field("digest_context", &self.digest_context)
            .field("replay_cache", &self.replay_cache)
            .field("credential_provider", &"<dyn CredentialProvider>")
            .field("tag_counter", &self.tag_counter)
            .finish()
    }
}

impl<P: CredentialProvider> Gb28181Downstream<P> {
    /// Creates a new downstream-platform handler.
    pub fn new(config: DownstreamConfig, credential_provider: P) -> Result<Self, DownstreamError> {
        let ctx = DigestContext::new(config.realm(), config.digest_secret().expose_secret())
            .map_err(|e| {
                DownstreamError::Access(crate::error::AccessError::Internal(e.to_string()))
            })?
            .allow_md5(config.allow_md5())
            .preferred_algorithm(config.preferred_algorithm())
            .qop(Some(DigestQop::Auth))
            .map_err(|e| {
                DownstreamError::Access(crate::error::AccessError::Internal(e.to_string()))
            })?;
        Ok(Self {
            links: LinkTable::new(config.max_links()),
            config,
            digest_context: ctx,
            replay_cache: DigestReplayCache::new(1024),
            credential_provider,
            tag_counter: AtomicU64::new(1),
        })
    }

    /// Processes a single input and returns ordered outputs.
    pub fn process(
        &mut self,
        input: DownstreamInput,
    ) -> Result<Vec<DownstreamOutput>, DownstreamError> {
        match input {
            DownstreamInput::Sip {
                source,
                now,
                message,
            } => {
                let method = match &message {
                    SipMessage::Request { line, .. } => Some(line.method.clone()),
                    SipMessage::Response { .. } => None,
                };
                match method {
                    Some(Method::Register) => register::process_register(
                        &self.config,
                        &self.digest_context,
                        &mut self.replay_cache,
                        &self.credential_provider,
                        &mut self.links,
                        &self.tag_counter,
                        source,
                        now,
                        message,
                    ),
                    Some(Method::Message) => Ok(message::process_message(
                        &self.config,
                        &mut self.links,
                        &self.tag_counter,
                        source,
                        now,
                        message,
                    )),
                    Some(_) => Ok(vec![DownstreamOutput::SendResponse(
                        build_method_not_allowed(&message, self.next_tag()),
                    )]),
                    None => Ok(Vec::new()),
                }
            }
            DownstreamInput::Command { now, command } => message::handle_command(
                &self.config,
                &mut self.links,
                &self.tag_counter,
                now,
                command,
            ),
        }
    }

    /// Advances timers and returns any resulting events.
    pub fn tick(&mut self, now: u64) -> Vec<DownstreamOutput> {
        let heartbeat_timeout = self.config.heartbeat_timeout_seconds();
        let mut outputs = Vec::new();
        let mut expired = Vec::new();

        for (platform_id, link) in self.links.iter_mut() {
            if now.saturating_sub(link.registered_at) >= link.expires as u64 {
                expired.push(platform_id.clone());
                outputs.push(DownstreamOutput::EmitEvent(
                    Gb28181Event::DeviceUnregistered {
                        domain_id: self.config.domain_id().clone(),
                        device_id: platform_id.clone(),
                        source: link.source,
                    },
                ));
                continue;
            }

            if !link.offline && now.saturating_sub(link.last_seen) >= heartbeat_timeout {
                link.offline = true;
                outputs.push(DownstreamOutput::EmitEvent(
                    Gb28181Event::DevicePresenceChanged {
                        domain_id: self.config.domain_id().clone(),
                        device_id: platform_id.clone(),
                        source: link.source,
                        presence: DevicePresence::Offline,
                    },
                ));
            }
        }

        for platform_id in expired {
            self.links.remove(&platform_id);
        }

        outputs
    }

    fn next_tag(&self) -> String {
        let n = self.tag_counter.fetch_add(1, Ordering::Relaxed);
        format!("gb{n}")
    }
}

fn build_method_not_allowed(request: &SipMessage, tag: String) -> SipMessage {
    use crate::access::{add_or_replace_tag, copy_common_headers};
    use cheetah_gb28181_core::{HeaderName, HeaderValue, StatusLine};

    let mut headers = copy_common_headers(request);
    if let Some(to) = request.headers().get(&HeaderName::To) {
        headers.append(
            HeaderName::To,
            HeaderValue::new(add_or_replace_tag(to.as_str(), &tag)),
        );
    }
    headers.append(HeaderName::ContentLength, HeaderValue::new("0"));
    SipMessage::Response {
        line: StatusLine::new(405, "Method Not Allowed"),
        headers,
        body: Vec::new(),
    }
}
