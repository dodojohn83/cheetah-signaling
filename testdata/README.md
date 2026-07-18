# Protocol Golden Test Data

This directory contains desensitized, synthetic protocol samples used by golden
and property tests.  Each sample is paired with:

- `*.meta.toml` describing source category, standard/profile, expected outcome
  and desensitization notes.
- `*.expected` containing the canonical normalized output produced by the
  in-tree parser/encoder.

All IDs, passwords, tokens, addresses and credentials are synthetic.  No real
deployment data, certificates or private keys are committed here.

Unless otherwise noted in a sample's `.meta.toml`, all samples in this directory
are released under the `MIT-0` license.

| Subdirectory | Protocol | Handled by |
| --- | --- | --- |
| `gb28181/sip/` | GB/T 28181 SIP datagrams | `crates/protocols/cheetah-gb28181-core/tests/golden.rs` |
| `gb28181/xml/` | GB/T 28181 MANSCDP/MANSRTSP XML | `crates/protocols/cheetah-gb28181-module/tests/golden_xml.rs` |
| `onvif/soap/` | ONVIF SOAP envelopes | `tools/onvif-simulator/src/golden_tests.rs` |
