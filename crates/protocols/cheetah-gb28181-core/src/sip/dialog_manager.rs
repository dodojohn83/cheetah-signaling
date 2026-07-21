//! Bounded dialog table that drives the [`Dialog`] state machines.
//!
//! [`DialogManager`] is the dialog-layer analogue of
//! [`crate::TransactionManager`]. It owns the RFC 3261 dialog state machines
//! keyed by [`DialogId`] and provides the glue required by
//! `03_architecture_transport_and_runtime.md`:
//!
//! - **in-dialog routing** — an in-dialog request or response is matched to its
//!   dialog by `Call-ID` + local/remote tags and routed through the dialog FSM
//!   so re-INVITE/BYE/INFO/SUBSCRIBE/NOTIFY use dialog route state rather than
//!   REGISTER routing;
//! - **duplicate / out-of-order absorption** — the dialog FSM drops in-dialog
//!   requests whose CSeq is not greater than the last seen value;
//! - **bounded size and TTL** — the dialog map never exceeds a configured
//!   capacity (least-recently-active entries are evicted) and terminated or
//!   idle dialogs are reclaimed after a configured time-to-live.
//!
//! Time is expressed as a monotonic [`Duration`]; callers supply `now` and the
//! manager never reads a clock itself.

use std::collections::HashMap;
use std::time::Duration;

use super::dialog::{Dialog, DialogEvent, DialogId, DialogOutput, extract_tag};
use crate::{HeaderName, SipError, SipMessage};

/// Default maximum number of live dialogs per shard/worker.
pub const DEFAULT_MAX_DIALOGS: usize = 4096;
/// Default idle/terminated time-to-live before a dialog is reclaimed.
pub const DEFAULT_DIALOG_TTL: Duration = Duration::from_secs(1800);

/// Configuration for a [`DialogManager`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DialogManagerConfig {
    /// Maximum number of live dialogs.
    pub max_dialogs: usize,
    /// Time-to-live after which idle or terminated dialogs are reclaimed.
    pub ttl: Duration,
}

impl Default for DialogManagerConfig {
    fn default() -> Self {
        Self {
            max_dialogs: DEFAULT_MAX_DIALOGS,
            ttl: DEFAULT_DIALOG_TTL,
        }
    }
}

/// Result of routing an in-dialog message through the manager.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DialogRouting {
    /// The message matched a live dialog and must be delivered to the TU.
    Deliver(SipMessage),
    /// The message matched a dialog that has now terminated; the entry has been
    /// removed. The message is still delivered to the TU (for example a BYE).
    Terminated(SipMessage),
    /// The message matched a dialog but was absorbed as a duplicate or
    /// out-of-order retransmission and must not be delivered again.
    Absorbed,
    /// No dialog matched the message; the caller handles it out-of-dialog.
    NoDialog(SipMessage),
    /// The dialog rejected the message as malformed.
    Failure(crate::SipErrorKind),
}

#[derive(Clone, Debug)]
struct Entry {
    dialog: Dialog,
    last_activity: Duration,
}

/// Owns the SIP dialogs for one shard/worker.
#[derive(Clone, Debug)]
pub struct DialogManager {
    config: DialogManagerConfig,
    dialogs: HashMap<DialogId, Entry>,
}

impl DialogManager {
    /// Creates a manager with the given configuration.
    pub fn new(config: DialogManagerConfig) -> Self {
        Self {
            config,
            dialogs: HashMap::new(),
        }
    }

    /// Number of live dialogs.
    pub fn len(&self) -> usize {
        self.dialogs.len()
    }

    /// Returns true when no dialogs are tracked.
    pub fn is_empty(&self) -> bool {
        self.dialogs.is_empty()
    }

    /// Returns true when a dialog with `id` is tracked.
    pub fn contains(&self, id: &DialogId) -> bool {
        self.dialogs.contains_key(id)
    }

    /// Establishes a UAS dialog from an incoming INVITE and the local tag placed
    /// in the 2xx final response, returning the new dialog id.
    pub fn establish_uas(
        &mut self,
        invite: &SipMessage,
        local_tag: impl Into<String>,
        now: Duration,
    ) -> Result<DialogId, SipError> {
        let dialog = Dialog::new_uas(invite, local_tag)?;
        Ok(self.insert(dialog, now))
    }

    /// Establishes a UAC dialog from the original INVITE and the received 2xx
    /// final response, returning the new dialog id.
    pub fn establish_uac(
        &mut self,
        invite: &SipMessage,
        final_response: &SipMessage,
        now: Duration,
    ) -> Result<DialogId, SipError> {
        let dialog = Dialog::new_uac(invite, final_response)?;
        Ok(self.insert(dialog, now))
    }

    fn insert(&mut self, dialog: Dialog, now: Duration) -> DialogId {
        let id = dialog.id().clone();
        self.ensure_capacity(now);
        self.dialogs.insert(
            id.clone(),
            Entry {
                dialog,
                last_activity: now,
            },
        );
        id
    }

    /// Computes the dialog id an inbound message belongs to, if the message
    /// carries the required `Call-ID` and both tags.
    ///
    /// For a received request the local tag is the `To` tag and the remote tag
    /// is the `From` tag; for a received response the roles are swapped.
    pub fn match_id(&self, message: &SipMessage) -> Option<DialogId> {
        let call_id = message.call_id()?.to_string();
        let from_tag = extract_tag(message.headers().get(&HeaderName::From)?.as_str())?;
        let to_tag = extract_tag(message.headers().get(&HeaderName::To)?.as_str())?;
        let (local_tag, remote_tag) = match message {
            SipMessage::Request { .. } => (to_tag, from_tag),
            SipMessage::Response { .. } => (from_tag, to_tag),
        };
        Some(DialogId {
            call_id,
            local_tag: local_tag.to_string(),
            remote_tag: remote_tag.to_string(),
        })
    }

    /// Routes an inbound in-dialog request through its dialog.
    pub fn handle_request(&mut self, message: SipMessage, now: Duration) -> DialogRouting {
        self.route(message, now, DialogEvent::Request)
    }

    /// Routes an inbound in-dialog response through its dialog.
    pub fn handle_response(&mut self, message: SipMessage, now: Duration) -> DialogRouting {
        self.route(message, now, DialogEvent::Response)
    }

    fn route(
        &mut self,
        message: SipMessage,
        now: Duration,
        to_event: impl FnOnce(SipMessage) -> DialogEvent,
    ) -> DialogRouting {
        let Some(id) = self.match_id(&message) else {
            return DialogRouting::NoDialog(message);
        };
        let Some(entry) = self.dialogs.get_mut(&id) else {
            return DialogRouting::NoDialog(message);
        };
        entry.last_activity = now;
        let outputs = entry.dialog.process(to_event(message));

        let mut delivered = None;
        let mut failure = None;
        let mut completed = false;
        for output in outputs {
            match output {
                DialogOutput::Deliver(msg) => delivered = Some(*msg),
                DialogOutput::Complete => completed = true,
                DialogOutput::Failure(kind) => failure = Some(kind),
            }
        }
        if completed {
            self.dialogs.remove(&id);
        }
        match (failure, delivered, completed) {
            (Some(kind), _, _) => DialogRouting::Failure(kind),
            (None, Some(msg), true) => DialogRouting::Terminated(msg),
            (None, Some(msg), false) => DialogRouting::Deliver(msg),
            (None, None, _) => DialogRouting::Absorbed,
        }
    }

    /// Reclaims terminated and idle dialogs.
    pub fn tick(&mut self, now: Duration) {
        let ttl = self.config.ttl;
        self.dialogs.retain(|_, entry| {
            !entry.dialog.is_terminated() && now.saturating_sub(entry.last_activity) < ttl
        });
    }

    /// Ensures room for one more dialog, evicting terminated dialogs first and
    /// then the least-recently-active dialog when still at capacity.
    fn ensure_capacity(&mut self, _now: Duration) {
        let max = self.config.max_dialogs;
        if max == 0 || self.dialogs.len() < max {
            return;
        }
        if let Some(id) = self
            .dialogs
            .iter()
            .find(|(_, e)| e.dialog.is_terminated())
            .map(|(k, _)| k.clone())
        {
            self.dialogs.remove(&id);
            return;
        }
        if let Some(id) = self
            .dialogs
            .iter()
            .min_by_key(|(_, e)| e.last_activity)
            .map(|(k, _)| k.clone())
        {
            self.dialogs.remove(&id);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{SipParser, SipParserConfig};

    fn parse(data: &str) -> SipMessage {
        SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default()).unwrap()
    }

    fn invite() -> SipMessage {
        parse(
            "INVITE sip:bob@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.1:5060;branch=z9hG4bKinv\r\n\
             From: <sip:alice@example.com>;tag=alice-tag\r\n\
             To: <sip:bob@example.com>\r\n\
             Contact: <sip:alice@192.0.2.1:5060>\r\n\
             Call-ID: dialog-call@example.com\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
        )
    }

    fn in_dialog(method: &str, cseq: u32) -> SipMessage {
        parse(&format!(
            "{method} sip:alice@192.0.2.1:5060 SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.2:5060;branch=z9hG4bK{method}{cseq}\r\n\
             From: <sip:bob@example.com>;tag=alice-tag\r\n\
             To: <sip:alice@example.com>;tag=bob-tag\r\n\
             Call-ID: dialog-call@example.com\r\n\
             CSeq: {cseq} {method}\r\n\
             Content-Length: 0\r\n\r\n"
        ))
    }

    fn manager() -> DialogManager {
        DialogManager::new(DialogManagerConfig::default())
    }

    #[test]
    fn establishes_uas_dialog_and_routes_in_dialog_request() {
        let mut mgr = manager();
        let id = mgr
            .establish_uas(&invite(), "bob-tag", Duration::ZERO)
            .unwrap();
        assert_eq!(mgr.len(), 1);
        assert!(mgr.contains(&id));

        // An in-dialog INFO is delivered to the TU.
        let routing = mgr.handle_request(in_dialog("INFO", 2), Duration::from_secs(1));
        assert!(matches!(routing, DialogRouting::Deliver(_)));
    }

    #[test]
    fn out_of_dialog_message_is_not_matched() {
        let mut mgr = manager();
        mgr.establish_uas(&invite(), "bob-tag", Duration::ZERO)
            .unwrap();
        // Same Call-ID but a different remote tag does not belong to the dialog.
        let stray = parse(
            "INFO sip:alice@192.0.2.1:5060 SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.2:5060;branch=z9hG4bKstray\r\n\
             From: <sip:bob@example.com>;tag=other-tag\r\n\
             To: <sip:alice@example.com>;tag=bob-tag\r\n\
             Call-ID: dialog-call@example.com\r\n\
             CSeq: 2 INFO\r\n\
             Content-Length: 0\r\n\r\n",
        );
        assert!(matches!(
            mgr.handle_request(stray, Duration::from_secs(1)),
            DialogRouting::NoDialog(_)
        ));
    }

    #[test]
    fn duplicate_and_out_of_order_requests_are_absorbed() {
        let mut mgr = manager();
        mgr.establish_uas(&invite(), "bob-tag", Duration::ZERO)
            .unwrap();
        assert!(matches!(
            mgr.handle_request(in_dialog("INFO", 5), Duration::from_secs(1)),
            DialogRouting::Deliver(_)
        ));
        // A lower/equal CSeq is a retransmission or reorder and is absorbed.
        assert!(matches!(
            mgr.handle_request(in_dialog("INFO", 5), Duration::from_secs(2)),
            DialogRouting::Absorbed
        ));
        assert!(matches!(
            mgr.handle_request(in_dialog("INFO", 3), Duration::from_secs(3)),
            DialogRouting::Absorbed
        ));
    }

    #[test]
    fn bye_terminates_and_removes_the_dialog() {
        let mut mgr = manager();
        mgr.establish_uas(&invite(), "bob-tag", Duration::ZERO)
            .unwrap();
        let routing = mgr.handle_request(in_dialog("BYE", 2), Duration::from_secs(1));
        assert!(matches!(routing, DialogRouting::Terminated(_)));
        assert_eq!(mgr.len(), 0);
    }

    #[test]
    fn capacity_is_bounded() {
        let mut mgr = DialogManager::new(DialogManagerConfig {
            max_dialogs: 1,
            ttl: DEFAULT_DIALOG_TTL,
        });
        mgr.establish_uas(&invite(), "bob-tag", Duration::ZERO)
            .unwrap();
        // A second, distinct dialog evicts the first under the capacity bound.
        let other = parse(
            "INVITE sip:bob@example.com SIP/2.0\r\n\
             Via: SIP/2.0/UDP 192.0.2.1:5060;branch=z9hG4bKinv2\r\n\
             From: <sip:carol@example.com>;tag=carol-tag\r\n\
             To: <sip:bob@example.com>\r\n\
             Contact: <sip:carol@192.0.2.9:5060>\r\n\
             Call-ID: other-call@example.com\r\n\
             CSeq: 1 INVITE\r\n\
             Content-Length: 0\r\n\r\n",
        );
        mgr.establish_uas(&other, "bob-tag2", Duration::from_secs(1))
            .unwrap();
        assert_eq!(mgr.len(), 1);
    }

    #[test]
    fn idle_dialogs_are_reclaimed_by_ttl() {
        let mut mgr = DialogManager::new(DialogManagerConfig {
            max_dialogs: 16,
            ttl: Duration::from_secs(10),
        });
        mgr.establish_uas(&invite(), "bob-tag", Duration::ZERO)
            .unwrap();
        mgr.tick(Duration::from_secs(5));
        assert_eq!(mgr.len(), 1);
        mgr.tick(Duration::from_secs(20));
        assert_eq!(mgr.len(), 0);
    }
}
