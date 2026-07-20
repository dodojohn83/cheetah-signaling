#![allow(missing_docs, clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use cheetah_signal_contracts::cheetah::media::v1::{
    MediaCommand, MediaControlPayload, MediaMutationContext, media_command::Command,
};
use prost::Message;

/// Encodes a non-negative integer as a Protobuf varint.
fn encode_varint(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (n & 0x7f) as u8;
        n >>= 7;
        if n != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if n == 0 {
            break;
        }
    }
    out
}

/// A new reader must decode an old-style message that omits recently added
/// header/context fields without error, using default values.
#[test]
fn new_reader_defaults_old_writer_omitted_fields() {
    let old_style = MediaCommand {
        target_media_node_instance_epoch: 0,
        context: Some(MediaMutationContext {
            contract_version: 0,
            ..Default::default()
        }),
        command: Some(Command::Control(MediaControlPayload {
            media_session_id: "session-1".to_string(),
            command_type: "noop".to_string(),
            payload: vec![],
        })),
    };

    let bytes = old_style.encode_to_vec();
    let decoded = MediaCommand::decode(&bytes[..]).expect("new reader decodes old writer");

    assert_eq!(decoded.target_media_node_instance_epoch, 0);
    assert!(decoded.context.is_some());
    assert_eq!(
        decoded.context.unwrap().contract_version,
        0,
        "newly added context field defaults to zero"
    );
}

/// A new reader must ignore unknown fields added by a future writer.
#[test]
fn new_reader_ignores_unknown_fields() {
    let mut bytes = MediaCommand {
        target_media_node_instance_epoch: 7,
        context: Some(MediaMutationContext {
            contract_version: 1,
            ..Default::default()
        }),
        command: Some(Command::Control(MediaControlPayload {
            media_session_id: "session-1".to_string(),
            command_type: "noop".to_string(),
            payload: vec![],
        })),
    }
    .encode_to_vec();

    // Append an unknown length-delimited field with field number 99.
    let unknown_payload = b"test";
    let tag = (99u64 << 3) | 2;
    bytes.extend(encode_varint(tag));
    bytes.extend(encode_varint(unknown_payload.len() as u64));
    bytes.extend_from_slice(unknown_payload);

    let decoded = MediaCommand::decode(&bytes[..]).expect("new reader ignores unknown fields");

    assert_eq!(decoded.target_media_node_instance_epoch, 7);
}
