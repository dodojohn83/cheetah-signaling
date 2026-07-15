//! SIP transaction identifier and branch validation policy.
//!
//! Transaction keys follow RFC 3261 Section 17.1/17.2: the key is derived from
//! the top `Via` `branch` parameter, the `CSeq` method, the `Call-ID`, and the
//! local role (UAC/UAS). The CSeq method is used rather than the request-line
//! method so that `ACK` for non-2xx responses and `CANCEL` remain associated with
//! the correct original INVITE transaction when required by later matching logic.

use crate::{Method, SipError, SipErrorKind, SipMessage};

/// Local SIP role used to determine whether a message belongs to a client or a
/// server transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum TransactionRole {
    /// User Agent Client: the local side initiated the request.
    Uac,
    /// User Agent Server: the local side received the request.
    Uas,
}

impl TransactionRole {
    /// Returns the opposite role.
    #[must_use]
    pub fn opposite(self) -> Self {
        match self {
            Self::Uac => Self::Uas,
            Self::Uas => Self::Uac,
        }
    }
}

/// Category of a SIP transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum TransactionKind {
    /// INVITE transaction, including its ACK (non-2xx) and CANCEL peers.
    Invite,
    /// Any non-INVITE request transaction.
    NonInvite,
}

/// Client or server half of a transaction.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum TransactionHalf {
    /// Client transaction at the UAC.
    Client,
    /// Server transaction at the UAS.
    Server,
}

/// Policy for validating the top `Via` `branch` parameter when building a
/// transaction key from a request.
///
/// RFC 3261 requires the branch to start with the magic cookie `z9hG4bK` for
/// requests. Some non-compliant devices omit it. This policy lets callers
/// explicitly opt into compatibility without weakening the default.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum BranchPolicy {
    /// Require RFC 3261 compliant `z9hG4bK` magic cookie on requests.
    #[default]
    Strict,
    /// Accept any non-empty branch value. Use only after explicit device
    /// compatibility configuration.
    Permissive,
}

/// RFC 3261 branch magic cookie.
const MAGIC_COOKIE: &str = "z9hG4bK";

/// A transaction key uniquely identifying a SIP transaction at one endpoint.
///
/// Two messages that belong to the same transaction will produce the same key
/// when observed from the same role. The key intentionally does not include the
/// CSeq number: retransmissions and responses share the same transaction.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct TransactionKey {
    branch: String,
    cseq_method: Method,
    call_id: String,
    role: TransactionRole,
}

impl TransactionKey {
    /// Attempts to build a transaction key from a message and local role.
    ///
    /// Returns `None` when a required component (`Via` branch, `Call-ID`, or
    /// `CSeq`) is missing or malformed. With `BranchPolicy::Strict`, a request
    /// whose top `Via` branch does not start with `z9hG4bK` is rejected.
    pub fn for_message(
        message: &SipMessage,
        role: TransactionRole,
        policy: BranchPolicy,
    ) -> Option<Self> {
        let branch = message.top_branch()?.to_string();
        if branch.is_empty() {
            return None;
        }

        let is_request = matches!(message, SipMessage::Request { .. });
        if is_request && policy == BranchPolicy::Strict && !branch.starts_with(MAGIC_COOKIE) {
            return None;
        }

        let call_id = message.call_id()?.to_string();
        let (_, cseq_method) = message.cseq()?;

        Some(Self {
            branch,
            cseq_method,
            call_id,
            role,
        })
    }

    /// Returns the top `Via` branch parameter.
    pub fn branch(&self) -> &str {
        &self.branch
    }

    /// Returns the `CSeq` method used in the transaction key.
    pub fn cseq_method(&self) -> &Method {
        &self.cseq_method
    }

    /// Returns the `Call-ID`.
    pub fn call_id(&self) -> &str {
        &self.call_id
    }

    /// Returns the local role for this transaction key.
    pub fn role(&self) -> TransactionRole {
        self.role
    }

    /// Returns whether this transaction is client- or server-side.
    pub fn half(&self) -> TransactionHalf {
        match self.role {
            TransactionRole::Uac => TransactionHalf::Client,
            TransactionRole::Uas => TransactionHalf::Server,
        }
    }

    /// Returns the transaction kind (INVITE or non-INVITE).
    pub fn kind(&self) -> TransactionKind {
        match self.cseq_method {
            Method::Invite => TransactionKind::Invite,
            _ => TransactionKind::NonInvite,
        }
    }

    /// Validates that the top `Via` branch complies with the given policy.
    ///
    /// This helper is exposed for callers that want to report a specific error
    /// when a non-compliant branch is encountered, rather than silently dropping
    /// the transaction key.
    pub fn validate_branch(branch: &str, policy: BranchPolicy) -> Result<(), SipError> {
        if branch.is_empty() {
            return Err(SipError::new(
                SipErrorKind::InvalidHeader,
                None,
                "Via branch is empty",
            ));
        }
        if policy == BranchPolicy::Strict && !branch.starts_with(MAGIC_COOKIE) {
            return Err(SipError::new(
                SipErrorKind::InvalidHeader,
                None,
                "Via branch missing RFC 3261 magic cookie",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::{SipParser, SipParserConfig};

    fn register_request() -> SipMessage {
        let data = "REGISTER sip:registrar.example.com SIP/2.0\r\n\
            Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bK123\r\n\
            From: <sip:alice@example.com>;tag=abc\r\n\
            To: <sip:alice@example.com>\r\n\
            Call-ID: call-1@example.com\r\n\
            CSeq: 1 REGISTER\r\n\
            Content-Length: 0\r\n\r\n";
        SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default()).unwrap()
    }

    fn invite_request() -> SipMessage {
        let data = "INVITE sip:bob@example.com SIP/2.0\r\n\
            Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKinvite\r\n\
            From: <sip:alice@example.com>;tag=abc\r\n\
            To: <sip:bob@example.com>\r\n\
            Call-ID: call-2@example.com\r\n\
            CSeq: 2 INVITE\r\n\
            Content-Length: 0\r\n\r\n";
        SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default()).unwrap()
    }

    fn ack_for_non_2xx() -> SipMessage {
        // ACK for a non-2xx response carries the INVITE branch and the INVITE
        // method in CSeq, but the request-line method is ACK.
        let data = "ACK sip:bob@example.com SIP/2.0\r\n\
            Via: SIP/2.0/UDP 192.168.1.1:5060;branch=z9hG4bKinvite\r\n\
            From: <sip:alice@example.com>;tag=abc\r\n\
            To: <sip:bob@example.com>\r\n\
            Call-ID: call-2@example.com\r\n\
            CSeq: 2 INVITE\r\n\
            Content-Length: 0\r\n\r\n";
        SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default()).unwrap()
    }

    fn non_compliant_branch_request() -> SipMessage {
        let data = "REGISTER sip:registrar.example.com SIP/2.0\r\n\
            Via: SIP/2.0/UDP 192.168.1.1:5060;branch=legacy123\r\n\
            From: <sip:alice@example.com>;tag=abc\r\n\
            To: <sip:alice@example.com>\r\n\
            Call-ID: call-3@example.com\r\n\
            CSeq: 1 REGISTER\r\n\
            Content-Length: 0\r\n\r\n";
        SipParser::parse_datagram(data.as_bytes(), SipParserConfig::default()).unwrap()
    }

    #[test]
    fn uac_client_key_for_request() {
        let msg = register_request();
        let key =
            TransactionKey::for_message(&msg, TransactionRole::Uac, BranchPolicy::Strict).unwrap();
        assert_eq!(key.branch(), "z9hG4bK123");
        assert_eq!(key.cseq_method(), &Method::Register);
        assert_eq!(key.call_id(), "call-1@example.com");
        assert_eq!(key.half(), TransactionHalf::Client);
        assert_eq!(key.kind(), TransactionKind::NonInvite);
        assert_eq!(key.role(), TransactionRole::Uac);
    }

    #[test]
    fn uas_server_key_for_request() {
        let msg = register_request();
        let key =
            TransactionKey::for_message(&msg, TransactionRole::Uas, BranchPolicy::Strict).unwrap();
        assert_eq!(key.half(), TransactionHalf::Server);
        assert_eq!(key.role(), TransactionRole::Uas);
    }

    #[test]
    fn invite_kind_and_ack_cseq_reuse() {
        let invite = invite_request();
        let ack = ack_for_non_2xx();
        let invite_key =
            TransactionKey::for_message(&invite, TransactionRole::Uac, BranchPolicy::Strict)
                .unwrap();
        let ack_key =
            TransactionKey::for_message(&ack, TransactionRole::Uac, BranchPolicy::Strict).unwrap();

        assert_eq!(invite_key.kind(), TransactionKind::Invite);
        assert_eq!(ack_key.cseq_method(), &Method::Invite);
        assert_eq!(invite_key, ack_key);
    }

    #[test]
    fn strict_policy_rejects_missing_magic_cookie() {
        let msg = non_compliant_branch_request();
        assert!(
            TransactionKey::for_message(&msg, TransactionRole::Uac, BranchPolicy::Strict).is_none()
        );
    }

    #[test]
    fn permissive_policy_accepts_missing_magic_cookie() {
        let msg = non_compliant_branch_request();
        let key = TransactionKey::for_message(&msg, TransactionRole::Uac, BranchPolicy::Permissive)
            .unwrap();
        assert_eq!(key.branch(), "legacy123");
    }

    #[test]
    fn validate_branch_reports_magic_cookie_error() {
        let result = TransactionKey::validate_branch("legacy123", BranchPolicy::Strict);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err.kind, SipErrorKind::InvalidHeader));
    }
}
