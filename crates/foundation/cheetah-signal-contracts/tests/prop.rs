//! Property-based / short-fuzz tests for protobuf contract decoding.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cheetah_signal_contracts::cheetah::common::v1::{
    CommandEnvelope, EnvelopeMeta, Uuid as UuidMsg,
};
use proptest::prelude::*;
use prost::Message;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// CommandEnvelope decode must not panic on arbitrary bytes.
    #[test]
    fn command_envelope_decode_never_panics(data in prop::collection::vec(any::<u8>(), 0..4096)) {
        let _ = CommandEnvelope::decode(data.as_slice());
    }

    /// A valid CommandEnvelope round-trips through encode/decode.
    #[test]
    fn command_envelope_round_trips(_seed in 0usize..1) {
        let msg = CommandEnvelope {
            meta: Some(EnvelopeMeta {
                correlation_id: Some(UuidMsg {
                    value: "corr-1".to_string(),
                }),
                ..EnvelopeMeta::default()
            }),
            target: None,
            idempotency_key: "key-1".to_string(),
            operation_id: "op-1".to_string(),
            step_id: "step-1".to_string(),
            command: None,
        };

        let encoded = msg.encode_to_vec();
        let decoded = CommandEnvelope::decode(encoded.as_slice()).expect("decode envelope");
        assert_eq!(msg, decoded);
    }
}
