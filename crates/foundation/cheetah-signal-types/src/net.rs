//! Network helpers for validation and SSRF protection.
//!
//! These functions operate only on `std::net` types and do not perform I/O.

/// Returns `true` if the address belongs to a network that should not be
/// reachable from an untrusted outbound request in production.
pub fn is_internal_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => is_internal_ipv4(v4),
        std::net::IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_internal_ipv4(v4);
            }
            v6.is_unspecified()
                || v6.is_loopback()
                || v6.is_unicast_link_local()
                || v6.is_unique_local()
        }
    }
}

fn is_internal_ipv4(v4: std::net::Ipv4Addr) -> bool {
    v4.is_unspecified() || v4.is_loopback() || v4.is_link_local() || v4.is_private()
}
