use std::{collections::BTreeSet, fmt::Display, net::IpAddr, time::Instant};

use url::Url;

use super::txt::TxtRecord;

/// Stable identity of one physical printer, across service types, subtypes,
/// network interfaces and address families.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PrinterKey {
    /// `DUUID`, the device UUID. Merges the several logical queues of one device.
    Device(String),
    /// `UUID`, the printer UUID. Identifies a service, not a device.
    Printer(String),
    /// DNS-SD instance label. A printer must advertise every service under the
    /// same instance name, so this is the documented fallback.
    Instance(String),
    /// Configured by hand rather than discovered.
    Static(String),
}

impl PrinterKey {
    pub fn from_txt(txt: &TxtRecord, instance: &str) -> Self {
        if let Some(uuid) = txt.device_uuid() {
            return PrinterKey::Device(uuid.to_ascii_lowercase());
        }
        if let Some(uuid) = txt.printer_uuid() {
            return PrinterKey::Printer(uuid.to_ascii_lowercase());
        }
        PrinterKey::Instance(instance.to_ascii_lowercase())
    }
}

impl Display for PrinterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            PrinterKey::Device(v) => write!(f, "device:{v}"),
            PrinterKey::Printer(v) => write!(f, "printer:{v}"),
            PrinterKey::Instance(v) => write!(f, "instance:{v}"),
            PrinterKey::Static(v) => write!(f, "static:{v}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrinterSource {
    Mdns,
    Static,
}

/// Three states rather than two, so consumers pick their own sensitivity instead
/// of inheriting ours. Wi-Fi printers power-save and routinely miss a multicast
/// round, so a single missed refresh must not read as offline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Availability {
    Online,
    /// Past the mDNS refresh window with a reconfirmation outstanding.
    Stale,
    Offline,
    /// Statically configured and not yet probed over IPP.
    Unknown,
}

impl Display for Availability {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let text = match self {
            Availability::Online => "online",
            Availability::Stale => "stale",
            Availability::Offline => "offline",
            Availability::Unknown => "unknown",
        };
        f.write_str(text)
    }
}

#[derive(Debug, Clone)]
pub struct Printer {
    pub key: PrinterKey,
    /// DNS-SD instance label, or the configured id for a static printer.
    pub name: String,
    pub source: PrinterSource,
    pub availability: Availability,
    pub uri: Url,
    pub host: String,
    pub port: u16,
    pub addresses: BTreeSet<IpAddr>,
    pub txt: TxtRecord,
    /// Advertised under `_print._sub`, the only discovery-time IPP Everywhere signal.
    pub ipp_everywhere: bool,
    /// Advertised under `_universal._sub`, or carrying a `URF` key.
    pub airprint: bool,
    pub last_seen: Instant,
}

impl Printer {
    pub fn is_secure(&self) -> bool {
        self.uri.scheme() == "ipps"
    }
}

#[derive(Debug, Clone)]
pub enum DiscoveryEvent {
    Added(Printer),
    Updated(Printer),
    AvailabilityChanged { printer: Printer, previous: Availability },
}
