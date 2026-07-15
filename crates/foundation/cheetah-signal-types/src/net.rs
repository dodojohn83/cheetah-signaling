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
    let [a, b, c, _d] = v4.octets();

    if v4.is_unspecified() || v4.is_loopback() || v4.is_link_local() || v4.is_private() {
        return true;
    }

    // 100.64.0.0/10 carrier-grade NAT
    if a == 100 && (64..=127).contains(&b) {
        return true;
    }

    // 192.0.0.0/24 and 192.0.2.0/24 documentation
    if a == 192 && b == 0 && c == 0 {
        return true;
    }
    if a == 192 && b == 0 && c == 2 {
        return true;
    }

    // 192.88.99.0/24 6to4 relay anycast
    if a == 192 && b == 88 && c == 99 {
        return true;
    }

    // 198.18.0.0/15 benchmarking
    if a == 198 && (18..=19).contains(&b) {
        return true;
    }

    // 198.51.100.0/24 documentation
    if a == 198 && b == 51 && c == 100 {
        return true;
    }

    // 203.0.113.0/24 documentation
    if a == 203 && b == 0 && c == 113 {
        return true;
    }

    false
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn loopback_and_private_are_internal() {
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(172, 16, 0, 1))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))));
    }

    #[test]
    fn reserved_and_documentation_ranges_are_internal() {
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(
            100, 127, 255, 254
        ))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(192, 0, 0, 1))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(192, 88, 99, 1))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 1))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 1))));
    }

    #[test]
    fn public_ipv4_is_not_internal() {
        assert!(!is_internal_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!is_internal_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_internal_ip(IpAddr::V4(Ipv4Addr::new(100, 63, 0, 1))));
        assert!(!is_internal_ip(IpAddr::V4(Ipv4Addr::new(100, 128, 0, 1))));
    }

    #[test]
    fn ipv4_mapped_ipv6_uses_internal_check() {
        assert!(is_internal_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_internal_ip("::ffff:192.168.1.1".parse().unwrap()));
        assert!(!is_internal_ip("::ffff:1.1.1.1".parse().unwrap()));
    }
}
