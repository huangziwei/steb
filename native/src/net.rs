//! `/proc/net/route`, read for a default route off this device.

use std::fs;

/// The kernel's IPv4 routing table.
const ROUTE_TABLE: &str = "/proc/net/route";

/// `RTF_UP`, in the Flags column.
const RTF_UP: u32 = 0x0001;

/// No default route in [`ROUTE_TABLE`]. False where the read fails.
pub fn is_offline() -> bool {
    fs::read_to_string(ROUTE_TABLE).is_ok_and(|table| !has_default_route(&table))
}

/// A `00000000` destination carrying [`RTF_UP`], in the whitespace-separated
/// columns `Iface Destination Gateway Flags …` after one header line.
fn has_default_route(table: &str) -> bool {
    table.lines().skip(1).any(|line| {
        let mut cols = line.split_whitespace().skip(1); // past Iface
        let destination = cols.next();
        let _gateway = cols.next();
        let flags = cols.next();
        destination == Some("00000000")
            && flags
                .and_then(|f| u32::from_str_radix(f, 16).ok())
                .is_some_and(|bits| bits & RTF_UP != 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A default route through the gateway, plus the on-link subnet.
    const CONNECTED: &str = "\
Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
wlan0\t00000000\t0101A8C0\t0003\t0\t0\t0\t00000000\t0\t0\t0
wlan0\t0001A8C0\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0";

    /// The on-link subnet alone.
    const NO_DEFAULT: &str = "\
Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT
wlan0\t0001A8C0\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0";

    #[test]
    fn a_default_route_means_reachable() {
        assert!(has_default_route(CONNECTED));
    }

    #[test]
    fn an_on_link_route_alone_is_not_a_way_out() {
        assert!(!has_default_route(NO_DEFAULT));
        assert!(!has_default_route(
            "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask"
        ));
    }

    #[test]
    fn a_default_route_without_rtf_up_does_not_count() {
        let down = CONNECTED.replace("00000000\t0101A8C0\t0003", "00000000\t0101A8C0\t0002");
        assert!(!has_default_route(&down));
    }

    #[test]
    fn columns_are_read_by_position() {
        let odd = "Iface\tDestination\tGateway \tFlags\n\
                   00000000\t0001A8C0\t00000000\t0001";
        assert!(!has_default_route(odd));
    }

    #[test]
    fn an_unreadable_table_is_not_an_answer() {
        assert!(!is_offline());
    }
}
