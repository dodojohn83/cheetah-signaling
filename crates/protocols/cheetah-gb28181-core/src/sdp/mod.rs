//! Sans-I/O SDP parser/encoder limited to the GB28181 subset.

pub mod encoder;
pub mod error;
pub mod parser;
pub mod session;

pub use encoder::encode_sdp;
pub use error::SdpError;
pub use parser::{SdpParserConfig, parse_sdp};
pub use session::{
    RtpMap, SdpAttribute, SdpConnection, SdpConnectionType, SdpDirection, SdpMedia, SdpOrigin,
    SdpSession, SdpSetup, SdpTime,
};
