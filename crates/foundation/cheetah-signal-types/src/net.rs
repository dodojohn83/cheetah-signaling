//! Network helpers for validation and SSRF protection.
//!
//! These functions operate only on `std::net` types and do not perform I/O.

/// Returns `true` if the address belongs to a network that should not be
/// reachable from an untrusted outbound request in production.
pub fn is_internal_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => is_internal_ipv4(v4),
        std::net::IpAddr::V6(v6) => is_internal_ipv6(v6),
    }
}

fn is_internal_ipv6(v6: std::net::Ipv6Addr) -> bool {
    if v6.is_unspecified() || v6.is_loopback() || v6.is_unicast_link_local() || v6.is_unique_local()
    {
        return true;
    }

    if let Some(v4) = v6.to_ipv4() {
        return is_internal_ipv4(v4);
    }

    let o = v6.octets();

    // 6to4 2002::/16
    if o[0] == 0x20 && o[1] == 0x02 {
        return true;
    }

    // Teredo 2001:0000::/32
    if o[0] == 0x20 && o[1] == 0x01 && o[2] == 0x00 && o[3] == 0x00 {
        return true;
    }

    // Documentation 2001:db8::/32
    if o[0] == 0x20 && o[1] == 0x01 && o[2] == 0x0d && o[3] == 0xb8 {
        return true;
    }

    false
}

fn is_internal_ipv4(v4: std::net::Ipv4Addr) -> bool {
    let [a, b, c, _d] = v4.octets();

    if v4.is_unspecified() || v4.is_loopback() || v4.is_link_local() || v4.is_private() {
        return true;
    }

    // 0.0.0.0/8 "This network"
    if a == 0 {
        return true;
    }

    // 240.0.0.0/4 reserved and 255.255.255.255 broadcast
    if v4.is_broadcast() || a >= 240 {
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
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(0, 255, 255, 255))));
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
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(
            255, 255, 255, 255
        ))));
        assert!(is_internal_ip(IpAddr::V4(Ipv4Addr::new(240, 0, 0, 1))));
    }

    #[test]
    fn public_ipv4_is_not_internal() {
        assert!(!is_internal_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
        assert!(!is_internal_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_internal_ip(IpAddr::V4(Ipv4Addr::new(100, 63, 0, 1))));
        assert!(!is_internal_ip(IpAddr::V4(Ipv4Addr::new(100, 128, 0, 1))));
    }

    #[test]
    fn ipv4_mapped_and_compatible_ipv6_use_internal_check() {
        assert!(is_internal_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(is_internal_ip("::ffff:192.168.1.1".parse().unwrap()));
        assert!(!is_internal_ip("::ffff:1.1.1.1".parse().unwrap()));
        assert!(is_internal_ip("::127.0.0.1".parse().unwrap()));
        assert!(is_internal_ip("::192.168.1.1".parse().unwrap()));
        assert!(!is_internal_ip("::1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn ipv6_internal_and_transition_addresses() {
        assert!(is_internal_ip("::1".parse().unwrap()));
        assert!(is_internal_ip("fe80::1".parse().unwrap()));
        assert!(is_internal_ip("fc00::1".parse().unwrap()));
        assert!(is_internal_ip("2002::1".parse().unwrap()));
        assert!(is_internal_ip(
            "2001:0000:4136:e378:8000:63bf:3fff:fdd2".parse().unwrap()
        ));
        assert!(is_internal_ip("2001:db8::1".parse().unwrap()));
    }

    #[test]
    fn public_ipv6_is_not_internal() {
        assert!(!is_internal_ip("2606:4700:4700::1111".parse().unwrap()));
        assert!(!is_internal_ip("2001:4860:4860::8888".parse().unwrap()));
    }
}
