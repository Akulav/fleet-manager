//! Wake-on-LAN: broadcast a magic packet for each known MAC of a PC. Works for
//! offline PCs, so it is sent by the master itself (no agent involved). The
//! PC's BIOS/NIC must have Wake-on-LAN enabled.

use std::net::UdpSocket;

/// Parse "AA:BB:CC:DD:EE:FF" / "AA-BB-..." / "AABBCC..." into 6 bytes.
pub fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 12 {
        return None;
    }
    let mut mac = [0u8; 6];
    for (i, b) in mac.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    if mac == [0; 6] || mac == [0xff; 6] {
        return None;
    }
    Some(mac)
}

pub fn magic_packet(mac: [u8; 6]) -> Vec<u8> {
    let mut p = vec![0xffu8; 6];
    for _ in 0..16 {
        p.extend_from_slice(&mac);
    }
    p
}

/// Send magic packets for every valid MAC to the global broadcast and to each
/// /24 broadcast derived from `ips`. Returns how many MACs were woken.
pub fn wake(macs: &[String], ips: &[String]) -> std::io::Result<usize> {
    let sock = UdpSocket::bind("0.0.0.0:0")?;
    sock.set_broadcast(true)?;
    let mut targets = vec!["255.255.255.255".to_string()];
    for ip in ips {
        if let Ok(std::net::IpAddr::V4(v4)) = ip.parse::<std::net::IpAddr>() {
            let o = v4.octets();
            if o[0] == 127 || o[0] == 169 {
                continue;
            }
            let b = format!("{}.{}.{}.255", o[0], o[1], o[2]);
            if !targets.contains(&b) {
                targets.push(b);
            }
        }
    }
    let mut woken = 0;
    for mac in macs.iter().filter_map(|m| parse_mac(m)) {
        let packet = magic_packet(mac);
        for t in &targets {
            for port in [9u16, 7] {
                let _ = sock.send_to(&packet, (t.as_str(), port));
            }
        }
        woken += 1;
    }
    Ok(woken)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_mac_formats() {
        let want = Some([0xaa, 0xbb, 0xcc, 0x01, 0x02, 0x03]);
        assert_eq!(parse_mac("AA:BB:CC:01:02:03"), want);
        assert_eq!(parse_mac("aa-bb-cc-01-02-03"), want);
        assert_eq!(parse_mac("aabbcc010203"), want);
        assert_eq!(parse_mac("00:00:00:00:00:00"), None);
        assert_eq!(parse_mac("nope"), None);
    }

    #[test]
    fn magic_packet_layout() {
        let p = magic_packet([1, 2, 3, 4, 5, 6]);
        assert_eq!(p.len(), 102);
        assert_eq!(&p[..6], &[0xff; 6]);
        assert_eq!(&p[96..], &[1, 2, 3, 4, 5, 6]);
    }
}
