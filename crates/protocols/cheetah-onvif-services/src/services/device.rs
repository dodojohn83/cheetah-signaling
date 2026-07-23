//! ONVIF Device service request builders and response parsers.

use crate::config::ParserLimits;
use crate::error::OnvifServiceError;
use crate::types::{CapabilityKind, CapabilityProbeResult, DeviceInformation, Service};
use cheetah_onvif_core::OnvifError;
use cheetah_onvif_core::discovery::XAddrPolicy;
use cheetah_onvif_core::services::system_date_time;
use cheetah_onvif_core::soap::Envelope;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};
use std::collections::HashMap;
use std::io::Cursor;

const DEVICE_NS: &str = "http://www.onvif.org/ver10/device/wsdl";

const GET_DEVICE_INFO_ACTION: &str = "http://www.onvif.org/ver10/device/wsdl/GetDeviceInformation";
const GET_SERVICES_ACTION: &str = "http://www.onvif.org/ver10/device/wsdl/GetServices";
const GET_CAPABILITIES_ACTION: &str = "http://www.onvif.org/ver10/device/wsdl/GetCapabilities";
const GET_HOSTNAME_ACTION: &str = "http://www.onvif.org/ver10/device/wsdl/GetHostname";
const GET_NETWORK_INTERFACES_ACTION: &str =
    "http://www.onvif.org/ver10/device/wsdl/GetNetworkInterfaces";

fn local_name(name: &quick_xml::name::QName<'_>) -> String {
    String::from_utf8_lossy(name.local_name().as_ref()).to_string()
}

fn limit_error(message: impl Into<String>) -> OnvifServiceError {
    OnvifServiceError::Onvif(OnvifError::LimitExceeded(message.into()))
}

/// Tracks parser limits while scanning an ONVIF response.
struct ParseContext<'a> {
    limits: &'a ParserLimits,
    stack: Vec<String>,
    text: String,
    node_count: usize,
}

impl<'a> ParseContext<'a> {
    fn new(limits: &'a ParserLimits, input: &str) -> Result<Self, OnvifServiceError> {
        if input.len() > limits.max_input_bytes {
            return Err(limit_error(format!(
                "response exceeds {} bytes",
                limits.max_input_bytes
            )));
        }
        Ok(Self {
            limits,
            stack: Vec::new(),
            text: String::new(),
            node_count: 0,
        })
    }

    fn on_start(&mut self, name: String) -> Result<(), OnvifServiceError> {
        self.node_count += 1;
        if self.node_count > self.limits.max_nodes {
            return Err(limit_error(format!(
                "response exceeds {} nodes",
                self.limits.max_nodes
            )));
        }
        if self.stack.len() + 1 > self.limits.max_depth {
            return Err(limit_error(format!(
                "response exceeds {} nesting depth",
                self.limits.max_depth
            )));
        }
        self.stack.push(name);
        self.text.clear();
        Ok(())
    }

    fn on_empty(&mut self) -> Result<(), OnvifServiceError> {
        self.node_count += 1;
        if self.node_count > self.limits.max_nodes {
            return Err(limit_error(format!(
                "response exceeds {} nodes",
                self.limits.max_nodes
            )));
        }
        if self.stack.len() + 1 > self.limits.max_depth {
            return Err(limit_error(format!(
                "response exceeds {} nesting depth",
                self.limits.max_depth
            )));
        }
        Ok(())
    }

    fn append_text(&mut self, s: &str) -> Result<(), OnvifServiceError> {
        if self.text.len() + s.len() > self.limits.max_text_bytes {
            return Err(limit_error(format!(
                "response text exceeds {} bytes",
                self.limits.max_text_bytes
            )));
        }
        self.text.push_str(s);
        Ok(())
    }

    fn on_end(&mut self) -> String {
        std::mem::take(&mut self.text)
    }

    fn pop(&mut self) {
        self.stack.pop();
    }

    fn parent(&self) -> Option<&str> {
        let len = self.stack.len();
        if len >= 2 {
            self.stack.get(len - 2).map(|s| s.as_str())
        } else {
            None
        }
    }
}

fn empty_body(name: &str) -> Result<String, OnvifServiceError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let mut element = BytesStart::new(name);
    element.push_attribute(("xmlns:tds", DEVICE_NS));
    writer.write_event(Event::Empty(element))?;
    String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifServiceError::Onvif(cheetah_onvif_core::OnvifError::Xml(e.to_string())))
}

/// Builds an unauthenticated `GetDeviceInformation` request.
pub fn get_device_information_request(
    message_id: impl Into<String>,
) -> Result<String, OnvifServiceError> {
    Envelope::new(
        GET_DEVICE_INFO_ACTION,
        empty_body("tds:GetDeviceInformation")?,
    )
    .with_message_id(message_id)
    .build()
    .map_err(OnvifServiceError::Onvif)
}

/// Parses a `GetDeviceInformationResponse`.
pub fn parse_get_device_information_response(
    xml: &str,
    limits: &ParserLimits,
) -> Result<DeviceInformation, OnvifServiceError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut info = DeviceInformation::default();
    let mut ctx = ParseContext::new(limits, xml)?;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(&e.name());
                ctx.on_start(name)?;
            }
            Ok(Event::Empty(_)) => {
                ctx.on_empty()?;
            }
            Ok(Event::Text(e)) => {
                ctx.append_text(&e.xml10_content().unwrap_or_default())?;
            }
            Ok(Event::End(e)) => {
                let parent = ctx.parent().map(|s| s.to_string());
                let name = local_name(&e.name());
                let text = ctx.on_end();
                if parent.as_deref() == Some("GetDeviceInformationResponse")
                    && name == "Manufacturer"
                {
                    info.manufacturer = text.trim().to_string();
                } else if parent.as_deref() == Some("GetDeviceInformationResponse")
                    && name == "Model"
                {
                    info.model = text.trim().to_string();
                } else if parent.as_deref() == Some("GetDeviceInformationResponse")
                    && name == "FirmwareVersion"
                {
                    info.firmware_version = text.trim().to_string();
                } else if parent.as_deref() == Some("GetDeviceInformationResponse")
                    && name == "SerialNumber"
                {
                    info.serial_number = text.trim().to_string();
                } else if parent.as_deref() == Some("GetDeviceInformationResponse")
                    && name == "HardwareId"
                {
                    info.hardware_id = text.trim().to_string();
                }
                ctx.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(OnvifServiceError::Onvif(
                    cheetah_onvif_core::OnvifError::Xml(e.to_string()),
                ));
            }
            _ => {}
        }
    }

    if info.manufacturer.is_empty() {
        return Err(OnvifServiceError::MissingField("Manufacturer".to_string()));
    }

    Ok(info)
}

/// Builds a `GetSystemDateAndTime` request.
pub fn get_system_date_and_time_request(
    message_id: impl Into<String>,
) -> Result<String, OnvifServiceError> {
    system_date_time::build_get_system_date_and_time(message_id).map_err(OnvifServiceError::Onvif)
}

/// Re-export of the core `SystemDateAndTime` parser.
pub use cheetah_onvif_core::services::system_date_time::{
    SystemDateAndTime, parse_get_system_date_and_time_response,
};

/// Builds a `GetServices` request.
pub fn get_services_request(
    include_capabilities: bool,
    message_id: impl Into<String>,
) -> Result<String, OnvifServiceError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);

    let mut body = BytesStart::new("tds:GetServices");
    body.push_attribute(("xmlns:tds", DEVICE_NS));
    writer.write_event(Event::Start(body))?;

    let mut flag = BytesStart::new("tds:IncludeCapability");
    flag.push_attribute(("xmlns:tds", DEVICE_NS));
    writer.write_event(Event::Start(flag))?;
    writer.write_event(Event::Text(quick_xml::events::BytesText::new(
        if include_capabilities {
            "true"
        } else {
            "false"
        },
    )))?;
    writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
        "tds:IncludeCapability",
    )))?;

    writer.write_event(Event::End(quick_xml::events::BytesEnd::new(
        "tds:GetServices",
    )))?;

    let body = String::from_utf8(cursor.into_inner())?;
    Envelope::new(GET_SERVICES_ACTION, body)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifServiceError::Onvif)
}

/// Parses a `GetServicesResponse` and validates each service XAddr against `policy`.
pub fn parse_get_services_response(
    xml: &str,
    limits: &ParserLimits,
    xaddr_policy: &XAddrPolicy,
) -> Result<Vec<Service>, OnvifServiceError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut services = Vec::new();
    let mut ctx = ParseContext::new(limits, xml)?;
    let mut current: Option<ServiceBuilder> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(&e.name());
                ctx.on_start(name)?;
            }
            Ok(Event::Empty(_)) => {
                ctx.on_empty()?;
            }
            Ok(Event::Text(e)) => {
                ctx.append_text(&e.xml10_content().unwrap_or_default())?;
            }
            Ok(Event::End(e)) => {
                let parent = ctx.parent().map(|s| s.to_string());
                let name = local_name(&e.name());
                let text = ctx.on_end();
                if name == "Service" {
                    if let Some(builder) = current.take() {
                        services.push(builder.build(xaddr_policy)?);
                    }
                } else if let Some(ref mut b) = current {
                    match name.as_str() {
                        "Namespace" if parent.as_deref() == Some("Service") => {
                            b.namespace = text.trim().to_string();
                        }
                        "XAddr" if parent.as_deref() == Some("Service") => {
                            b.xaddr = text.trim().to_string();
                        }
                        "Version" if parent.as_deref() == Some("Service") => {
                            if b.major.is_some() || b.minor.is_some() {
                                b.version = b.version_or_default();
                            } else {
                                b.version = text.trim().to_string();
                            }
                        }
                        "Major" if parent.as_deref() == Some("Version") => {
                            b.major = Some(text.trim().to_string());
                        }
                        "Minor" if parent.as_deref() == Some("Version") => {
                            b.minor = Some(text.trim().to_string());
                        }
                        _ => {}
                    }
                } else if name == "Namespace" && parent.as_deref() == Some("Service") {
                    current = Some(ServiceBuilder {
                        namespace: text.trim().to_string(),
                        xaddr: String::new(),
                        version: String::new(),
                        major: None,
                        minor: None,
                    });
                }
                ctx.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(OnvifServiceError::Onvif(
                    cheetah_onvif_core::OnvifError::Xml(e.to_string()),
                ));
            }
            _ => {}
        }
    }

    Ok(services)
}

#[derive(Default)]
struct ServiceBuilder {
    namespace: String,
    xaddr: String,
    version: String,
    major: Option<String>,
    minor: Option<String>,
}

impl ServiceBuilder {
    fn version_or_default(&self) -> String {
        match (&self.major, &self.minor) {
            (Some(major), Some(minor)) => format!("{major}.{minor}"),
            _ if !self.version.is_empty() => self.version.clone(),
            _ => String::new(),
        }
    }

    fn build(self, xaddr_policy: &XAddrPolicy) -> Result<Service, OnvifServiceError> {
        if self.namespace.is_empty() {
            return Err(OnvifServiceError::MissingField("Namespace".to_string()));
        }
        if self.xaddr.is_empty() {
            return Err(OnvifServiceError::MissingField("XAddr".to_string()));
        }
        let url = url::Url::parse(&self.xaddr)?;
        xaddr_policy.validate(&url)?;
        let version = self.version_or_default();
        Ok(Service {
            namespace: self.namespace,
            xaddr: self.xaddr,
            version,
        })
    }
}

/// Builds a `GetCapabilities` request.
pub fn get_capabilities_request(
    message_id: impl Into<String>,
) -> Result<String, OnvifServiceError> {
    Envelope::new(GET_CAPABILITIES_ACTION, empty_body("tds:GetCapabilities")?)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifServiceError::Onvif)
}

/// Parses a `GetCapabilitiesResponse` into high-level capability results.
pub fn parse_get_capabilities_response(
    xml: &str,
    limits: &ParserLimits,
) -> Result<HashMap<CapabilityKind, CapabilityProbeResult>, OnvifServiceError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut caps: HashMap<CapabilityKind, CapabilityProbeResult> = HashMap::new();
    let mut ctx = ParseContext::new(limits, xml)?;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(&e.name());
                check_capability(&mut caps, &name);
                ctx.on_start(name)?;
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(&e.name());
                check_capability(&mut caps, &name);
                ctx.on_empty()?;
            }
            Ok(Event::End(_e)) => {
                let _ = ctx.on_end();
                ctx.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(OnvifServiceError::Onvif(
                    cheetah_onvif_core::OnvifError::Xml(e.to_string()),
                ));
            }
            _ => {}
        }
    }

    Ok(caps)
}

fn check_capability(caps: &mut HashMap<CapabilityKind, CapabilityProbeResult>, tag: &str) {
    if let Some(kind) = capability_kind_from_tag(tag) {
        caps.entry(kind)
            .or_insert(CapabilityProbeResult::Supported {
                namespace: namespace_for_kind(kind).to_string(),
                xaddr: None,
                version: None,
            });
    }
}

fn capability_kind_from_tag(tag: &str) -> Option<CapabilityKind> {
    match tag {
        "Device" => Some(CapabilityKind::Device),
        "Media" | "Media2" => Some(CapabilityKind::Media),
        "PTZ" => Some(CapabilityKind::Ptz),
        "Events" => Some(CapabilityKind::Events),
        "Imaging" => Some(CapabilityKind::Imaging),
        "Analytics" => Some(CapabilityKind::Analytics),
        "Extension" => Some(CapabilityKind::Extension),
        _ => None,
    }
}

fn namespace_for_kind(kind: CapabilityKind) -> &'static str {
    match kind {
        CapabilityKind::Device => "http://www.onvif.org/ver10/device/wsdl",
        CapabilityKind::Media => "http://www.onvif.org/ver10/media/wsdl",
        CapabilityKind::Ptz => "http://www.onvif.org/ver20/ptz/wsdl",
        CapabilityKind::Events => "http://www.onvif.org/ver10/events/wsdl",
        CapabilityKind::Imaging => "http://www.onvif.org/ver20/imaging/wsdl",
        CapabilityKind::Analytics => "http://www.onvif.org/ver20/analytics/wsdl",
        CapabilityKind::Extension => "",
    }
}

/// Builds a `GetHostname` request.
pub fn get_hostname_request(message_id: impl Into<String>) -> Result<String, OnvifServiceError> {
    Envelope::new(GET_HOSTNAME_ACTION, empty_body("tds:GetHostname")?)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifServiceError::Onvif)
}

/// Builds a `GetNetworkInterfaces` request.
pub fn get_network_interfaces_request(
    message_id: impl Into<String>,
) -> Result<String, OnvifServiceError> {
    Envelope::new(
        GET_NETWORK_INTERFACES_ACTION,
        empty_body("tds:GetNetworkInterfaces")?,
    )
    .with_message_id(message_id)
    .build()
    .map_err(OnvifServiceError::Onvif)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn limits() -> ParserLimits {
        ParserLimits {
            max_depth: 64,
            max_nodes: 1024,
            max_text_bytes: 1024,
            max_input_bytes: 65_536,
        }
    }

    fn policy() -> XAddrPolicy {
        XAddrPolicy::default()
    }

    #[test]
    fn get_device_information_request_contains_action() -> Result<(), OnvifServiceError> {
        let xml = get_device_information_request("urn:uuid:1")?;
        assert!(xml.contains(GET_DEVICE_INFO_ACTION));
        assert!(xml.contains("GetDeviceInformation"));
        assert!(xml.contains("urn:uuid:1"));
        assert!(xml.contains("xmlns:tds"));
        Ok(())
    }

    #[test]
    fn parses_device_information_response() -> Result<(), OnvifServiceError> {
        let xml = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
  <s:Body>
    <tds:GetDeviceInformationResponse xmlns:tds="http://www.onvif.org/ver10/device/wsdl">
      <tt:Manufacturer xmlns:tt="http://www.onvif.org/ver10/schema">Acme</tt:Manufacturer>
      <tt:Model xmlns:tt="http://www.onvif.org/ver10/schema">Cam-1</tt:Model>
      <tt:FirmwareVersion xmlns:tt="http://www.onvif.org/ver10/schema">1.0</tt:FirmwareVersion>
      <tt:SerialNumber xmlns:tt="http://www.onvif.org/ver10/schema">SN123</tt:SerialNumber>
      <tt:HardwareId xmlns:tt="http://www.onvif.org/ver10/schema">HW1</tt:HardwareId>
    </tds:GetDeviceInformationResponse>
  </s:Body>
</s:Envelope>"#;
        let info = parse_get_device_information_response(xml, &limits())?;
        assert_eq!(info.manufacturer, "Acme");
        assert_eq!(info.model, "Cam-1");
        assert_eq!(info.firmware_version, "1.0");
        assert_eq!(info.serial_number, "SN123");
        assert_eq!(info.hardware_id, "HW1");
        Ok(())
    }

    #[test]
    fn get_services_request_contains_include_capability() -> Result<(), OnvifServiceError> {
        let xml = get_services_request(true, "urn:uuid:2")?;
        assert!(xml.contains(GET_SERVICES_ACTION));
        assert!(xml.contains("GetServices"));
        assert!(xml.contains("IncludeCapability"));
        assert!(xml.contains("true"));
        Ok(())
    }

    #[test]
    fn parses_services_response() -> Result<(), OnvifServiceError> {
        let xml = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
  <s:Body>
    <tds:GetServicesResponse xmlns:tds="http://www.onvif.org/ver10/device/wsdl">
      <tds:Service>
        <tds:Namespace>http://www.onvif.org/ver10/device/wsdl</tds:Namespace>
        <tds:XAddr>http://192.0.2.1/onvif/device_service</tds:XAddr>
        <tds:Version>1.0</tds:Version>
      </tds:Service>
      <tds:Service>
        <tds:Namespace>http://www.onvif.org/ver10/media/wsdl</tds:Namespace>
        <tds:XAddr>http://192.0.2.1/onvif/media_service</tds:XAddr>
        <tds:Version>1.0</tds:Version>
      </tds:Service>
    </tds:GetServicesResponse>
  </s:Body>
</s:Envelope>"#;
        let services = parse_get_services_response(xml, &limits(), &policy())?;
        assert_eq!(services.len(), 2);
        assert_eq!(
            services[0].namespace,
            "http://www.onvif.org/ver10/device/wsdl"
        );
        assert_eq!(services[0].xaddr, "http://192.0.2.1/onvif/device_service");
        Ok(())
    }

    #[test]
    fn parses_services_response_with_major_minor_version() -> Result<(), OnvifServiceError> {
        let xml = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
  <s:Body>
    <tds:GetServicesResponse xmlns:tds="http://www.onvif.org/ver10/device/wsdl">
      <tds:Service>
        <tds:Namespace>http://www.onvif.org/ver10/device/wsdl</tds:Namespace>
        <tds:XAddr>http://192.0.2.1/onvif/device_service</tds:XAddr>
        <tds:Version>
          <tt:Major xmlns:tt="http://www.onvif.org/ver10/schema">2</tt:Major>
          <tt:Minor xmlns:tt="http://www.onvif.org/ver10/schema">60</tt:Minor>
        </tds:Version>
      </tds:Service>
    </tds:GetServicesResponse>
  </s:Body>
</s:Envelope>"#;
        let services = parse_get_services_response(xml, &limits(), &policy())?;
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].version, "2.60");
        Ok(())
    }

    #[test]
    fn parser_rejects_ssrf_xaddr() {
        let xml = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
  <s:Body>
    <tds:GetServicesResponse xmlns:tds="http://www.onvif.org/ver10/device/wsdl">
      <tds:Service>
        <tds:Namespace>http://www.onvif.org/ver10/device/wsdl</tds:Namespace>
        <tds:XAddr>http://127.0.0.1/onvif/device_service</tds:XAddr>
        <tds:Version>1.0</tds:Version>
      </tds:Service>
    </tds:GetServicesResponse>
  </s:Body>
</s:Envelope>"#;
        assert!(parse_get_services_response(xml, &limits(), &policy()).is_err());
    }

    #[test]
    fn parses_capabilities_response() -> Result<(), OnvifServiceError> {
        let xml = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
  <s:Body>
    <tds:GetCapabilitiesResponse xmlns:tds="http://www.onvif.org/ver10/device/wsdl">
      <tds:Capabilities>
        <tt:Device xmlns:tt="http://www.onvif.org/ver10/schema"/>
        <tt:Media xmlns:tt="http://www.onvif.org/ver10/schema"/>
        <tt:PTZ xmlns:tt="http://www.onvif.org/ver10/schema"/>
      </tds:Capabilities>
    </tds:GetCapabilitiesResponse>
  </s:Body>
</s:Envelope>"#;
        let caps = parse_get_capabilities_response(xml, &limits())?;
        assert!(caps.contains_key(&CapabilityKind::Device));
        assert!(caps.contains_key(&CapabilityKind::Media));
        assert!(caps.contains_key(&CapabilityKind::Ptz));
        Ok(())
    }

    #[test]
    fn parser_rejects_oversized_input() {
        let small_limits = ParserLimits {
            max_input_bytes: 10,
            ..ParserLimits::default()
        };
        assert!(parse_get_device_information_response("<root></root>", &small_limits).is_err());
    }

    #[test]
    fn parser_rejects_deep_nesting() {
        let shallow_limits = ParserLimits {
            max_depth: 2,
            ..ParserLimits::default()
        };
        let xml = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
  <s:Body>
    <tds:GetDeviceInformationResponse xmlns:tds="http://www.onvif.org/ver10/device/wsdl">
      <tds:Information>
        <tt:Manufacturer xmlns:tt="http://www.onvif.org/ver10/schema">Acme</tt:Manufacturer>
      </tds:Information>
    </tds:GetDeviceInformationResponse>
  </s:Body>
</s:Envelope>"#;
        assert!(parse_get_device_information_response(xml, &shallow_limits).is_err());
    }
}
