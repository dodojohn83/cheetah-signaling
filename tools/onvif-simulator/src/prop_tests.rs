//! Property-based / short-fuzz tests for the ONVIF simulator SOAP parser.

use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// parse_envelope must not panic on arbitrary bytes.
    #[test]
    fn parse_envelope_never_panics(data in prop::collection::vec(any::<u8>(), 0..4096)) {
        let _ = super::parse_envelope(&data);
    }
}
