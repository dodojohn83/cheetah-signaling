//! Golden tests for GB28181 MANSCDP/MANSRTSP XML samples under
//! `testdata/gb28181/xml/`.
//!
//! Each `<sample>.xml` is parsed with the in-tree reader and re-encoded with
//! `encode_xml`. The canonical output is stored in `<sample>.expected`. Run with
//! `UPDATE_GOLDEN=1` to regenerate `.expected` files.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};

use cheetah_gb28181_module::xml::{XmlLimits, encode_xml, parse_xml};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../")
        .canonicalize()
        .expect("repo root")
}

fn normalize(data: &[u8]) -> String {
    let root = parse_xml(data, &XmlLimits::default()).expect("sample should parse");
    encode_xml(&root, true).expect("sample should encode")
}

#[test]
fn golden_xml_samples() {
    let samples_dir = repo_root().join("testdata/gb28181/xml");
    let update = std::env::var("UPDATE_GOLDEN").is_ok();
    let mut checked = 0;

    for entry in fs::read_dir(&samples_dir).expect("read samples dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("xml") {
            continue;
        }

        let data = fs::read(&path).expect("read sample");
        let normalized = normalize(&data);
        let expected_path = path.with_extension("expected");

        if update {
            fs::write(&expected_path, normalized).expect("write expected");
        } else {
            let expected = fs::read_to_string(&expected_path)
                .unwrap_or_else(|_| panic!("missing expected file: {}", expected_path.display()));
            assert_eq!(
                normalized,
                expected,
                "golden mismatch for {}",
                path.file_name().unwrap().to_string_lossy()
            );
        }
        checked += 1;
    }

    assert!(
        checked > 0,
        "no .xml samples found in {}",
        samples_dir.display()
    );
}
