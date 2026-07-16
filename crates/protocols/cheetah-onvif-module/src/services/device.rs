//! ONVIF Device service request builders and response parsers.

use crate::error::OnvifModuleError;
use crate::types::{CapabilityKind, CapabilityProbeResult, DeviceInformation, Service};
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

fn empty_body(name: &str) -> Result<String, OnvifModuleError> {
    let mut cursor = Cursor::new(Vec::new());
    let mut writer = Writer::new(&mut cursor);
    let element = BytesStart::new(name);
    writer.write_event(Event::Empty(element))?;
    String::from_utf8(cursor.into_inner())
        .map_err(|e| OnvifModuleError::Onvif(cheetah_onvif_core::OnvifError::Xml(e.to_string())))
}

/// Builds an unauthenticated `GetDeviceInformation` request.
pub fn get_device_information_request(
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
    Envelope::new(
        GET_DEVICE_INFO_ACTION,
        empty_body("tds:GetDeviceInformation")?,
    )
    .with_message_id(message_id)
    .build()
    .map_err(OnvifModuleError::Onvif)
}

/// Parses a `GetDeviceInformationResponse`.
pub fn parse_get_device_information_response(
    xml: &str,
) -> Result<DeviceInformation, OnvifModuleError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut info = DeviceInformation::default();
    let mut context: Vec<String> = Vec::new();
    let mut text = String::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                context.push(local_name(&e.name()));
                text.clear();
            }
            Ok(Event::Empty(_)) => {
                text.clear();
            }
            Ok(Event::Text(e)) => {
                text.push_str(&e.xml10_content().unwrap_or_default());
            }
            Ok(Event::End(e)) => {
                let name = local_name(&e.name());
                if context.last().is_some_and(|c| c == "Manufacturer") && name == "Manufacturer" {
                    info.manufacturer = text.trim().to_string();
                } else if context.last().is_some_and(|c| c == "Model") && name == "Model" {
                    info.model = text.trim().to_string();
                } else if context.last().is_some_and(|c| c == "FirmwareVersion")
                    && name == "FirmwareVersion"
                {
                    info.firmware_version = text.trim().to_string();
                } else if context.last().is_some_and(|c| c == "SerialNumber")
                    && name == "SerialNumber"
                {
                    info.serial_number = text.trim().to_string();
                } else if context.last().is_some_and(|c| c == "HardwareId") && name == "HardwareId"
                {
                    info.hardware_id = text.trim().to_string();
                }
                context.pop();
                text.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(OnvifModuleError::Onvif(
                    cheetah_onvif_core::OnvifError::Xml(e.to_string()),
                ));
            }
            _ => {}
        }
    }

    if info.manufacturer.is_empty() {
        return Err(OnvifModuleError::MissingField("Manufacturer".to_string()));
    }

    Ok(info)
}

/// Builds a `GetSystemDateAndTime` request.
pub fn get_system_date_and_time_request(
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
    system_date_time::build_get_system_date_and_time(message_id).map_err(OnvifModuleError::Onvif)
}

/// Re-export of the core `SystemDateAndTime` parser.
pub use cheetah_onvif_core::services::system_date_time::{
    SystemDateAndTime, parse_get_system_date_and_time_response,
};

/// Builds a `GetServices` request.
pub fn get_services_request(
    include_capabilities: bool,
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
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
        .map_err(OnvifModuleError::Onvif)
}

/// Parses a `GetServicesResponse`.
pub fn parse_get_services_response(xml: &str) -> Result<Vec<Service>, OnvifModuleError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut services = Vec::new();
    let mut context: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut current: Option<ServiceBuilder> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                context.push(local_name(&e.name()));
                text.clear();
            }
            Ok(Event::Empty(_)) => {
                text.clear();
            }
            Ok(Event::Text(e)) => {
                text.push_str(&e.xml10_content().unwrap_or_default());
            }
            Ok(Event::End(e)) => {
                let name = local_name(&e.name());
                if name == "Service" && context.last().is_some_and(|c| c == "Service") {
                    if let Some(builder) = current.take() {
                        services.push(builder.build()?);
                    }
                } else if let Some(ref mut b) = current {
                    match name.as_str() {
                        "Namespace" => b.namespace = text.trim().to_string(),
                        "XAddr" => b.xaddr = text.trim().to_string(),
                        "Version" => b.version = text.trim().to_string(),
                        _ => {}
                    }
                } else if name == "Namespace" && context.last().is_some_and(|c| c == "Namespace") {
                    current = Some(ServiceBuilder {
                        namespace: text.trim().to_string(),
                        xaddr: String::new(),
                        version: String::new(),
                    });
                }
                context.pop();
                text.clear();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(OnvifModuleError::Onvif(
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
}

impl ServiceBuilder {
    fn build(self) -> Result<Service, OnvifModuleError> {
        if self.namespace.is_empty() {
            return Err(OnvifModuleError::MissingField("Namespace".to_string()));
        }
        if self.xaddr.is_empty() {
            return Err(OnvifModuleError::MissingField("XAddr".to_string()));
        }
        Ok(Service {
            namespace: self.namespace,
            xaddr: self.xaddr,
            version: self.version,
        })
    }
}

/// Builds a `GetCapabilities` request.
pub fn get_capabilities_request(message_id: impl Into<String>) -> Result<String, OnvifModuleError> {
    Envelope::new(GET_CAPABILITIES_ACTION, empty_body("tds:GetCapabilities")?)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifModuleError::Onvif)
}

/// Parses a `GetCapabilitiesResponse` into high-level capability results.
pub fn parse_get_capabilities_response(
    xml: &str,
) -> Result<HashMap<CapabilityKind, CapabilityProbeResult>, OnvifModuleError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut caps: HashMap<CapabilityKind, CapabilityProbeResult> = HashMap::new();
    let mut context: Vec<String> = Vec::new();

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(&e.name());
                check_capability(&mut caps, &name);
                context.push(name);
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(&e.name());
                check_capability(&mut caps, &name);
            }
            Ok(Event::End(_)) => {
                context.pop();
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(OnvifModuleError::Onvif(
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
pub fn get_hostname_request(message_id: impl Into<String>) -> Result<String, OnvifModuleError> {
    Envelope::new(GET_HOSTNAME_ACTION, empty_body("tds:GetHostname")?)
        .with_message_id(message_id)
        .build()
        .map_err(OnvifModuleError::Onvif)
}

/// Builds a `GetNetworkInterfaces` request.
pub fn get_network_interfaces_request(
    message_id: impl Into<String>,
) -> Result<String, OnvifModuleError> {
    Envelope::new(
        GET_NETWORK_INTERFACES_ACTION,
        empty_body("tds:GetNetworkInterfaces")?,
    )
    .with_message_id(message_id)
    .build()
    .map_err(OnvifModuleError::Onvif)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_device_information_request_contains_action() -> Result<(), OnvifModuleError> {
        let xml = get_device_information_request("urn:uuid:1")?;
        assert!(xml.contains(GET_DEVICE_INFO_ACTION));
        assert!(xml.contains("GetDeviceInformation"));
        assert!(xml.contains("urn:uuid:1"));
        Ok(())
    }

    #[test]
    fn parses_device_information_response() -> Result<(), OnvifModuleError> {
        let xml = r#"<?xml version="1.0"?>
<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope">
  <s:Body>
    <tds:GetDeviceInformationResponse xmlns:tds="http://www.onvif.org/ver10/device/wsdl">
      <tds:Information>
        <tt:Manufacturer xmlns:tt="http://www.onvif.org/ver10/schema">Acme</tt:Manufacturer>
        <tt:Model xmlns:tt="http://www.onvif.org/ver10/schema">Cam-1</tt:Model>
        <tt:FirmwareVersion xmlns:tt="http://www.onvif.org/ver10/schema">1.0</tt:FirmwareVersion>
        <tt:SerialNumber xmlns:tt="http://www.onvif.org/ver10/schema">SN123</tt:SerialNumber>
        <tt:HardwareId xmlns:tt="http://www.onvif.org/ver10/schema">HW1</tt:HardwareId>
      </tds:Information>
    </tds:GetDeviceInformationResponse>
  </s:Body>
</s:Envelope>"#;
        let info = parse_get_device_information_response(xml)?;
        assert_eq!(info.manufacturer, "Acme");
        assert_eq!(info.model, "Cam-1");
        assert_eq!(info.firmware_version, "1.0");
        assert_eq!(info.serial_number, "SN123");
        assert_eq!(info.hardware_id, "HW1");
        Ok(())
    }

    #[test]
    fn get_services_request_contains_include_capability() -> Result<(), OnvifModuleError> {
        let xml = get_services_request(true, "urn:uuid:2")?;
        assert!(xml.contains(GET_SERVICES_ACTION));
        assert!(xml.contains("GetServices"));
        assert!(xml.contains("IncludeCapability"));
        assert!(xml.contains("true"));
        Ok(())
    }

    #[test]
    fn parses_services_response() -> Result<(), OnvifModuleError> {
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
        let services = parse_get_services_response(xml)?;
        assert_eq!(services.len(), 2);
        assert_eq!(
            services[0].namespace,
            "http://www.onvif.org/ver10/device/wsdl"
        );
        assert_eq!(services[0].xaddr, "http://192.0.2.1/onvif/device_service");
        Ok(())
    }

    #[test]
    fn parses_capabilities_response() -> Result<(), OnvifModuleError> {
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
        let caps = parse_get_capabilities_response(xml)?;
        assert!(caps.contains_key(&CapabilityKind::Device));
        assert!(caps.contains_key(&CapabilityKind::Media));
        assert!(caps.contains_key(&CapabilityKind::Ptz));
        Ok(())
    }
}
