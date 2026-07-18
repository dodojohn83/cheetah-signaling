//! Property-based / short-fuzz tests for the GB28181 MANSCDP/MANSRTSP XML codec.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use cheetah_gb28181_module::xml::{XmlLimits, build_keepalive, parse_keepalive, parse_xml};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// The XML reader must not panic on arbitrary bytes and must respect limits.
    #[test]
    fn xml_reader_never_panics(data in prop::collection::vec(any::<u8>(), 0..4096)) {
        let _ = parse_xml(&data, &XmlLimits::default());
    }

    /// Building and parsing a Keepalive XML preserves the field values.
    #[test]
    fn keepalive_round_trips(
        sn in "[0-9a-zA-Z]{1,16}",
        device_id in "[0-9]{1,20}",
        status in "[A-Za-z0-9]{1,8}",
    ) {
        let xml = build_keepalive(&sn, &device_id, &status).expect("build keepalive");
        let info = parse_keepalive(xml.as_bytes()).expect("parse keepalive");
        assert_eq!(info.sn, sn);
        assert_eq!(info.device_id, device_id);
        assert_eq!(info.status, status);
    }

    /// XML beyond the configured depth limit is rejected.
    #[test]
    fn xml_rejects_excessive_depth(depth in 5usize..10usize) {
        let mut body = String::new();
        for _ in 0..depth {
            body.push_str("<a>");
        }
        body.push('x');
        for _ in 0..depth {
            body.push_str("</a>");
        }
        let result = parse_xml(body.as_bytes(), &XmlLimits::test());
        assert!(result.is_err());
    }

    /// XML beyond the configured children-per-element limit is rejected.
    #[test]
    fn xml_rejects_excessive_children(count in 9usize..16usize) {
        let mut body = String::from("<root>");
        for i in 0..count {
            body.push_str(&format!("<c>{i}</c>"));
        }
        body.push_str("</root>");
        let result = parse_xml(body.as_bytes(), &XmlLimits::test());
        assert!(result.is_err());
    }
}
