//! Property-based tests for foundation types.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cheetah_signal_types::{ListCursor, UtcTimestamp};
use proptest::prelude::*;
use uuid::Uuid;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// ListCursor::decode must not panic on arbitrary strings.
    #[test]
    fn list_cursor_decode_never_panics(value in any::<String>()) {
        let _ = ListCursor::decode(&value);
    }

    /// ListCursor round-trips through encode/decode for valid values.
    #[test]
    fn list_cursor_round_trips(_seed in 0usize..1) {
        let ts = UtcTimestamp::from_offset(time::OffsetDateTime::UNIX_EPOCH);
        let id = Uuid::from_u128(1);
        let cursor = ListCursor::new(ts, id).expect("create cursor");
        let encoded = cursor.encode().expect("encode cursor");
        let decoded = ListCursor::decode(&encoded).expect("decode cursor");
        assert_eq!(cursor, decoded);
    }
}
