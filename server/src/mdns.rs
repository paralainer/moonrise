//! Bonjour advertising: the server announces itself on the local network as `_moonrise._tcp`
//! (mDNS, with host networking), so the app finds it without an address. The TXT record carries
//! the server id (as `GET /v1/info` answers it), the version and the API path, and `setup=unclaimed`
//! while the server has no accounts (`claim.rs`).
//!
//! Only the LAN interfaces take part: Docker bridges, veths, VPNs (Tailscale) and loopback are
//! left out, so the addresses announced are ones a phone on the Wi-Fi can reach.
//! `MOONRISE_MDNS_INTERFACES=eno1,…` names them instead. Kill switch: `MOONRISE_DISABLE=mdns`.
//!
//! On macOS the system's mDNSResponder announces it (`DNSServiceRegister`): apps on the same Mac
//! see it too (a second responder's packets don't loop back to the first), and a name another
//! server already uses is renamed ("Moonrise Server (2)").

use std::net::{IpAddr, Ipv4Addr};

#[cfg(not(target_os = "macos"))]
use anyhow::Context;
use anyhow::Result;
#[cfg(not(target_os = "macos"))]
use mdns_sd::{IfKind, ServiceDaemon, ServiceInfo};

pub const SERVICE_TYPE: &str = "_moonrise._tcp.local.";

/// Interfaces that never carry the LAN: container bridges, VPNs, loopback.
pub fn is_virtual(name: &str) -> bool {
    [
        "lo",
        "docker",
        "br-",
        "veth",
        "virbr",
        "tailscale",
        "tun",
        "wg",
        "zt",
        "cni",
        "flannel",
        "vxlan",
        "lxc",
    ]
    .iter()
    .any(|p| name.starts_with(p))
}

fn is_private(ip: Ipv4Addr) -> bool {
    ip.is_private() && !ip.is_loopback()
}

/// (interface, address) pairs to announce on.
fn lan_interfaces(names: Option<&str>) -> Vec<(String, Ipv4Addr)> {
    let wanted: Option<Vec<&str>> = names.map(|n| n.split(',').map(str::trim).filter(|s| !s.is_empty()).collect());
    let mut out: Vec<(String, Ipv4Addr)> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|i| match i.ip() {
            IpAddr::V4(v4) => Some((i.name, v4)),
            IpAddr::V6(_) => None,
        })
        .filter(|(name, ip)| match &wanted {
            Some(w) => w.contains(&name.as_str()),
            None => !is_virtual(name) && is_private(*ip),
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The addresses a phone on the Wi-Fi can reach this computer at (as Bonjour announces them).
pub fn lan_addresses() -> Vec<Ipv4Addr> {
    let names = std::env::var("MOONRISE_MDNS_INTERFACES").ok().filter(|s| !s.trim().is_empty());
    lan_interfaces(names.as_deref()).into_iter().map(|(_, ip)| ip).collect()
}

/// A host name for the A records: the server's name made DNS-safe, plus a piece of its id.
#[cfg_attr(target_os = "macos", allow(dead_code, reason = "macOS registers through dns_sd, without these"))]
fn host_name(name: &str, server_id: &str) -> String {
    let mut base: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    base = base.trim_matches('-').to_string();
    if base.is_empty() {
        base = "moonrise".into();
    }
    let short: String = server_id.chars().filter(char::is_ascii_alphanumeric).take(6).collect();
    format!("{base}-{short}.local.")
}

/// The Bonjour instance name where nothing renames on a conflict: "Moonrise Server (6bc367)".
#[cfg_attr(target_os = "macos", allow(dead_code, reason = "macOS registers through dns_sd, without these"))]
fn instance_name(name: &str, server_id: &str) -> String {
    let short: String = server_id.chars().filter(char::is_ascii_alphanumeric).take(6).collect();
    format!("{} ({short})", name.trim())
}

/// A running announcement (the daemon runs on its own thread until [`Announcement::stop`]).
#[cfg(not(target_os = "macos"))]
pub struct Announcement {
    daemon: ServiceDaemon,
    fullname: String,
}

#[cfg(not(target_os = "macos"))]
impl Announcement {
    /// Says goodbye on the network and stops the daemon.
    pub fn stop(self) {
        if let Ok(rx) = self.daemon.unregister(&self.fullname) {
            let _ = rx.recv_timeout(std::time::Duration::from_secs(2));
        }
        let _ = self.daemon.shutdown();
    }
}

/// A registration with the system's mDNSResponder (macOS); dropping it withdraws the service.
#[cfg(target_os = "macos")]
pub struct Announcement {
    reg: dnssd::Registration,
}

#[cfg(target_os = "macos")]
impl Announcement {
    pub fn stop(self) {
        drop(self.reg);
    }
}

/// The TXT record: length-prefixed `key=value` strings.
#[cfg_attr(
    not(target_os = "macos"),
    allow(dead_code, reason = "only the dns_sd registration on macOS uses it")
)]
fn txt_record(props: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (k, v) in props {
        let kv = format!("{k}={v}");
        let b = &kv.as_bytes()[..kv.len().min(255)];
        out.push(b.len() as u8);
        out.extend_from_slice(b);
    }
    out
}

#[cfg(target_os = "macos")]
mod dnssd {
    use std::ffi::{c_char, c_void, CString};

    use anyhow::{bail, Result};

    type DNSServiceRef = *mut c_void;

    // dns_sd.h, in libSystem.
    extern "C" {
        fn DNSServiceRegister(
            sd_ref: *mut DNSServiceRef,
            flags: u32,
            interface_index: u32,
            name: *const c_char,
            regtype: *const c_char,
            domain: *const c_char,
            host: *const c_char,
            port: u16,
            txt_len: u16,
            txt: *const c_void,
            callback: *const c_void,
            context: *mut c_void,
        ) -> i32;
        fn DNSServiceRefDeallocate(sd_ref: DNSServiceRef);
    }

    pub struct Registration(DNSServiceRef);

    // SAFETY: the DNSServiceRef is owned by the Registration and only used again to deallocate
    // it, once, in Drop; dns_sd allows a reference to be used from any thread, one at a time.
    unsafe impl Send for Registration {}

    impl Drop for Registration {
        fn drop(&mut self) {
            // SAFETY: a Registration is only made from a successful DNSServiceRegister (a non-null
            // reference), and this is the only place it is deallocated.
            unsafe { DNSServiceRefDeallocate(self.0) }
        }
    }

    /// Registers `name` as `_moonrise._tcp` on every interface, renamed by the daemon on a conflict
    /// (no callback: nothing to hear back).
    pub fn register(name: &str, port: u16, txt: &[u8]) -> Result<Registration> {
        let name = CString::new(name)?;
        let regtype = CString::new("_moonrise._tcp")?;
        let mut r: DNSServiceRef = std::ptr::null_mut();
        // SAFETY: every pointer is valid for the call: `r` is a local out-parameter, `name` and
        // `regtype` are NUL-terminated and outlive it, `txt` points at the length given (at most
        // that many bytes are read), and the nulls are what dns_sd documents for the default
        // domain and host, no callback and no context.
        let err = unsafe {
            DNSServiceRegister(
                &mut r,
                0,
                0,
                name.as_ptr(),
                regtype.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                port.to_be(),
                txt.len() as u16,
                txt.as_ptr() as *const c_void,
                std::ptr::null(),
                std::ptr::null_mut(),
            )
        };
        if err != 0 || r.is_null() {
            bail!("DNSServiceRegister failed ({err})");
        }
        Ok(Registration(r))
    }
}

/// Start announcing (macOS: through mDNSResponder).
#[cfg(target_os = "macos")]
pub fn advertise(
    name: &str,
    server_id: &str,
    version: &str,
    port: u16,
    _interfaces: Option<&str>,
    unclaimed: bool,
) -> Result<Announcement> {
    let mut props = vec![("id", server_id), ("version", version), ("path", "/v1/info")];
    if unclaimed {
        props.push(("setup", "unclaimed"));
    }
    let reg = dnssd::register(name, port, &txt_record(&props))?;
    tracing::info!(
        service = SERVICE_TYPE,
        name,
        port,
        unclaimed,
        "announced on the local network (mDNSResponder)"
    );
    Ok(Announcement { reg })
}

/// Start announcing.
#[cfg(not(target_os = "macos"))]
pub fn advertise(name: &str, server_id: &str, version: &str, port: u16, interfaces: Option<&str>, unclaimed: bool) -> Result<Announcement> {
    let lan = lan_interfaces(interfaces);
    anyhow::ensure!(!lan.is_empty(), "no LAN interface to announce on");
    let daemon = ServiceDaemon::new().context("mDNS daemon")?;
    daemon.disable_interface(IfKind::All).ok();
    for (iface, _) in &lan {
        daemon.enable_interface(IfKind::Name(iface.clone())).ok();
    }
    let ips: Vec<IpAddr> = lan.iter().map(|(_, ip)| IpAddr::V4(*ip)).collect();
    let mut props = vec![("id", server_id), ("version", version), ("path", "/v1/info")];
    if unclaimed {
        // Not set up yet: the app goes straight to the setup code.
        props.push(("setup", "unclaimed"));
    }
    let host = host_name(name, server_id);
    // mdns-sd doesn't rename on a conflict, and two servers under one name hide each other (their
    // records merge): the instance carries a piece of the id. The apps show `/v1/info`'s name.
    let instance = instance_name(name, server_id);
    let info = ServiceInfo::new(SERVICE_TYPE, &instance, &host, &ips[..], port, &props[..]).context("mDNS service")?;
    let fullname = info.get_fullname().to_string();
    daemon.register(info).context("mDNS register")?;
    tracing::info!(service = SERVICE_TYPE, name = instance, host, interfaces = ?lan, port, unclaimed, "announced on the local network");
    Ok(Announcement { daemon, fullname })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_interfaces_are_skipped() {
        for n in ["lo", "docker0", "br-e57464d46e10", "veth12ab", "tailscale0", "wg0"] {
            assert!(is_virtual(n), "{n}");
        }
        for n in ["eno1", "eth0", "enp3s0", "wlan0", "wlp2s0"] {
            assert!(!is_virtual(n), "{n}");
        }
    }

    #[test]
    fn host_names_are_dns_safe() {
        assert_eq!(host_name("Moonrise Server", "6bc36742-dad1-454d"), "moonrise-server-6bc367.local.");
        assert_eq!(host_name("  ", "x"), "moonrise-x.local.");
        assert_eq!(instance_name("Moonrise Server", "6bc36742-dad1-454d"), "Moonrise Server (6bc367)");
    }

    #[test]
    fn txt_records_are_length_prefixed() {
        assert_eq!(
            txt_record(&[("id", "ab"), ("setup", "unclaimed")]),
            b"\x05id=ab\x0fsetup=unclaimed".to_vec()
        );
    }

    #[test]
    fn private_addresses_only() {
        assert!(is_private("192.168.1.10".parse().unwrap()));
        assert!(!is_private("100.64.0.1".parse().unwrap()));
        assert!(!is_private("127.0.0.1".parse().unwrap()));
    }
}
