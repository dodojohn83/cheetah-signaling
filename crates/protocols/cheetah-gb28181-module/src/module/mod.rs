//! Sans-I/O GB28181 device-access state machine.

use crate::{Gb28181ModuleError, config::Gb28181Config};
use cheetah_gb28181_core::{DigestContext, DigestReplayCache, Method, SipMessage};
use cheetah_runtime_api::DeviceKey;
use cheetah_signal_types::{MessageId, UtcTimestamp};
use secrecy::ExposeSecret;
use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::sync::Arc;

mod catalog;
mod message;
mod register;

/// Input to the module.
#[derive(Clone, Debug)]
pub struct Gb28181Input {
    /// Source endpoint of the SIP message.
    pub source: SocketAddr,
    /// Parsed SIP message.
    pub message: SipMessage,
}

/// Per-device GB28181 state machine.
#[derive(Debug)]
#[allow(dead_code)]
pub struct Gb28181Module {
    device_key: DeviceKey,
    config: Arc<Gb28181Config>,
    digest: DigestContext,
    replay_cache: DigestReplayCache,
    registration: Option<Registration>,
    pending_commands: BTreeMap<u32, PendingCommand>,
    catalog: Option<CatalogAggregator>,
    next_cseq: u32,
    next_sn: u32,
    recent_message_ids: BTreeSet<(String, u32)>,
    recent_messages: std::collections::VecDeque<RecentMessage>,
}

const MESSAGE_DEDUP_WINDOW_SECONDS: u64 = 60;

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct RecentMessage {
    call_id: String,
    cseq: u32,
    seen_at: u64,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct Registration {
    external_id: String,
    endpoint: SocketAddr,
    call_id: String,
    from_tag: String,
    expires_seconds: u32,
    authenticated: bool,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct PendingCommand {
    command_id: MessageId,
    sent_at: UtcTimestamp,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
struct CatalogAggregator {
    sn: u32,
    sum_num: u32,
    items: Vec<crate::output::Gb28181CatalogItem>,
    received_fragments: u32,
}

impl Gb28181Module {
    /// Creates a new module instance for the given device key and config.
    pub fn new(
        device_key: DeviceKey,
        config: Arc<Gb28181Config>,
    ) -> Result<Self, Gb28181ModuleError> {
        let secret = config
            .auth_policy
            .server_secret
            .expose_secret()
            .as_bytes()
            .to_vec();
        let digest = DigestContext::new(&config.realm, secret)?;
        let digest = if config.auth_policy.allow_md5 {
            digest.allow_md5(true)
        } else {
            digest
        };
        Ok(Self {
            device_key,
            config,
            digest,
            replay_cache: DigestReplayCache::new(1024),
            registration: None,
            pending_commands: BTreeMap::new(),
            catalog: None,
            next_cseq: 1,
            next_sn: 1,
            recent_message_ids: BTreeSet::new(),
            recent_messages: std::collections::VecDeque::new(),
        })
    }

    /// Returns the device key.
    pub fn device_key(&self) -> DeviceKey {
        self.device_key
    }

    /// Returns a reference to the GB28181 configuration.
    pub fn config(&self) -> &Gb28181Config {
        &self.config
    }

    /// Handles one input and returns outputs.
    pub fn handle(
        &mut self,
        input: Gb28181Input,
        now: UtcTimestamp,
    ) -> Result<Vec<crate::output::Gb28181Output>, Gb28181ModuleError> {
        match &input.message {
            SipMessage::Request { line, .. } => match &line.method {
                Method::Register => self.handle_register(input.source, input.message, now),
                Method::Message => Ok(self.handle_message(input.source, input.message, now)),
                Method::Options => Ok(vec![crate::output::Gb28181Output::SendMessage {
                    endpoint: input.source,
                    message: message::ok_response(&input.message),
                }]),
                Method::Invite | Method::Ack | Method::Bye | Method::Cancel => {
                    Ok(vec![crate::output::Gb28181Output::SendMessage {
                        endpoint: input.source,
                        message: message::simple_response(&input.message, 501, "Not Implemented"),
                    }])
                }
                _ => Ok(vec![crate::output::Gb28181Output::SendMessage {
                    endpoint: input.source,
                    message: message::simple_response(&input.message, 405, "Method Not Allowed"),
                }]),
            },
            SipMessage::Response { .. } => {
                Ok(self.handle_response(input.source, input.message, now))
            }
        }
    }

    /// Handles an incoming domain command and produces a SIP request, if any.
    ///
    /// Outgoing control-plane commands (PTZ, playback control) require a
    /// channel-id-to-external-id mapping that is not yet available at this
    /// layer; the current implementation therefore returns an explicit
    /// `ProtocolError::Unsupported` output.
    pub fn handle_command(
        &mut self,
        command: &cheetah_domain::Command,
        _now: UtcTimestamp,
    ) -> Result<Vec<crate::output::Gb28181Output>, Gb28181ModuleError> {
        let source = self.registration.as_ref().map(|r| r.endpoint);
        Ok(vec![crate::output::Gb28181Output::ProtocolError {
            source,
            kind: "unsupported_command".into(),
            message: format!("{} command not supported yet", command.payload().kind()),
        }])
    }

    /// Called when a heartbeat timer fires.
    pub fn heartbeat_timeout(&mut self, _now: UtcTimestamp) -> Vec<crate::output::Gb28181Output> {
        self.registration = None;
        vec![crate::output::Gb28181Output::ProtocolError {
            source: None,
            kind: "heartbeat_timeout".into(),
            message: "heartbeat timeout; clearing registration".into(),
        }]
    }

    /// Resets module state on ownership change.
    pub fn reset(&mut self) {
        self.registration = None;
        self.pending_commands.clear();
        self.catalog = None;
        self.recent_message_ids.clear();
        self.recent_messages.clear();
    }

    /// Test seam to register a pending command for `DeviceControl` response handling.
    pub fn add_pending_command(&mut self, command_id: MessageId, sent_at: UtcTimestamp) {
        let sn = self.next_sn;
        self.next_sn += 1;
        self.pending_commands.insert(
            sn,
            PendingCommand {
                command_id,
                sent_at,
            },
        );
    }
}
