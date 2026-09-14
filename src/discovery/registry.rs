use std::{collections::{BTreeSet, HashMap}, net::IpAddr, time::{Duration, Instant}};

use log::{debug, warn};
use mdns_sd::ResolvedService;
use url::Url;

use crate::config::models::{Discovery, StaticPrinter};

use super::{models::*, txt::TxtRecord};

const IPP_PARENT_TYPES: [(&str, &str); 2] = [("._ipps._tcp.local.", "ipps"), ("._ipp._tcp.local.", "ipp")];

/// Holds every known printer and owns identity, merging and availability.
pub struct Registry {
    printers: HashMap<PrinterKey, Printer>,
    /// Service instance fullname to printer key, so a removal can be attributed
    /// to the printer it belongs to.
    instances: HashMap<String, PrinterKey>,
    stale_after: Duration,
    offline_after: Duration,
    ipp_everywhere_only: bool,
}

impl Registry {
    pub fn new(settings: &Discovery, static_printers: &[StaticPrinter]) -> Self {
        let mut registry = Registry {
            printers: HashMap::new(),
            instances: HashMap::new(),
            stale_after: settings.stale_after,
            offline_after: settings.offline_after,
            ipp_everywhere_only: settings.ipp_everywhere_only,
        };

        for configured in static_printers {
            match Printer::from_static(configured) {
                Ok(printer) => { registry.printers.insert(printer.key.clone(), printer); }
                Err(error) => warn!("Ignoring static printer [{}]: {error}", configured.id),
            }
        }

        registry
    }

    pub fn printers(&self) -> impl Iterator<Item = &Printer> {
        self.printers.values()
    }

    /// Folds a resolved service into the registry. The same instance resolves
    /// repeatedly as more A/AAAA records arrive, so this upserts and never treats
    /// a re-resolve as an arrival.
    pub fn resolved(&mut self, service: &ResolvedService) -> Option<DiscoveryEvent> {
        let fresh = match Printer::from_service(service) {
            Ok(printer) => printer,
            Err(error) => {
                debug!("Ignoring service [{}]: {error}", service.fullname);
                return None;
            }
        };

        if self.ipp_everywhere_only && !fresh.ipp_everywhere {
            debug!("Ignoring [{}]: does not advertise the IPP Everywhere subtype", fresh.name);
            return None;
        }

        let key = fresh.key.clone();
        self.instances.insert(service.fullname.clone(), key.clone());

        match self.printers.get_mut(&key) {
            Some(existing) => {
                let previous = existing.availability;
                let changed = existing.absorb(fresh);
                match previous {
                    Availability::Online if changed => Some(DiscoveryEvent::Updated(existing.clone())),
                    Availability::Online => None,
                    _ => Some(DiscoveryEvent::AvailabilityChanged { printer: existing.clone(), previous }),
                }
            }
            None => {
                self.printers.insert(key.clone(), fresh);
                Some(DiscoveryEvent::Added(self.printers[&key].clone()))
            }
        }
    }

    /// Handles a goodbye or cache expiry for one service instance. A printer that
    /// drops `_ipp` while still answering `_ipps` is not gone, and neither is one
    /// that simply missed a multicast round, so this never removes anything; the
    /// sweep decides on elapsed time.
    ///
    /// Returns the instance fullname when it is worth reconfirming over mDNS.
    pub fn removed(&mut self, fullname: &str) -> Option<String> {
        let key = self.instances.remove(fullname)?;
        if self.instances.values().any(|other| *other == key) {
            debug!("Instance [{fullname}] went away but [{key}] still has another service");
            return None;
        }
        Some(fullname.to_owned())
    }

    /// Ages printers towards stale and then offline. Driven by elapsed time rather
    /// than by events, so a single missed refresh cannot flap the state.
    pub fn sweep(&mut self) -> Vec<DiscoveryEvent> {
        let mut events = Vec::new();

        for printer in self.printers.values_mut() {
            if printer.source == PrinterSource::Static {
                continue;
            }

            let elapsed = printer.last_seen.elapsed();
            let next = if elapsed > self.offline_after {
                Availability::Offline
            } else if elapsed > self.stale_after {
                Availability::Stale
            } else {
                Availability::Online
            };

            if next != printer.availability {
                let previous = printer.availability;
                printer.availability = next;
                events.push(DiscoveryEvent::AvailabilityChanged { printer: printer.clone(), previous });
            }
        }

        events
    }
}

impl Printer {
    fn from_service(service: &ResolvedService) -> Result<Self, PrinterError> {
        let (suffix, scheme) = IPP_PARENT_TYPES
            .iter()
            .find(|(suffix, _)| service.fullname.ends_with(suffix))
            .ok_or(PrinterError::NotAnIppService)?;

        // The scheme comes from the fullname, never from ty_domain: when browsing a
        // subtype, ty_domain is the subtype string and an ipps printer silently
        // downgrades to ipp.
        let name = service.fullname.strip_suffix(suffix).unwrap_or(&service.fullname).to_owned();
        if service.port == 0 {
            return Err(PrinterError::PlaceholderService);
        }

        let txt = TxtRecord::from(&service.txt_properties);
        let host = service.host.trim_end_matches('.').to_owned();
        let uri = build_uri(scheme, &host, service.port, txt.resource_path())?;

        // A printer advertising several subtypes only ever gets one of them back in
        // `sub_ty_domain`, because the daemon cache holds a single subtype per
        // instance. `ty_domain` is the subtype the delivering browse matched, so
        // both are checked and the flags latch across events in `absorb`.
        let subtypes = [service.ty_domain.as_str(), service.sub_ty_domain.as_deref().unwrap_or_default()];
        let advertises = |subtype: &str| subtypes.iter().any(|candidate| candidate.starts_with(subtype));

        Ok(Printer {
            key: PrinterKey::from_txt(&txt, &name),
            name,
            source: PrinterSource::Mdns,
            availability: Availability::Online,
            uri,
            host,
            port: service.port,
            addresses: service.addresses.iter().map(|address| address.to_ip_addr()).collect(),
            ipp_everywhere: advertises("_print._sub."),
            airprint: advertises("_universal._sub.") || !txt.urf().is_empty(),
            txt,
            last_seen: Instant::now(),
        })
    }

    fn from_static(configured: &StaticPrinter) -> Result<Self, PrinterError> {
        let uri = Url::parse(&configured.uri).map_err(|_| PrinterError::InvalidUri { uri: configured.uri.clone() })?;
        if uri.scheme() != "ipp" && uri.scheme() != "ipps" {
            return Err(PrinterError::UnsupportedScheme { scheme: uri.scheme().to_owned() });
        }

        let host = uri.host_str().ok_or(PrinterError::InvalidUri { uri: configured.uri.clone() })?.to_owned();
        let port = uri.port().unwrap_or(631);
        let addresses = host.parse::<IpAddr>().map(|address| BTreeSet::from([address])).unwrap_or_default();

        Ok(Printer {
            key: PrinterKey::Static(configured.id.clone()),
            name: configured.id.clone(),
            source: PrinterSource::Static,
            availability: Availability::Unknown,
            uri,
            host,
            port,
            addresses,
            txt: TxtRecord::default(),
            ipp_everywhere: false,
            airprint: false,
            last_seen: Instant::now(),
        })
    }

    /// Folds a freshly resolved view of the same printer into this one. Addresses
    /// union, TXT records merge, and an `ipps` service wins over `ipp`.
    ///
    /// Returns whether anything other than `last_seen` changed, so a re-resolve
    /// that carries no new information stays silent.
    fn absorb(&mut self, fresh: Printer) -> bool {
        let upgrading = fresh.is_secure() && !self.is_secure();
        let before = (self.addresses.len(), self.txt.clone(), self.ipp_everywhere, self.airprint, self.uri.clone());

        self.last_seen = fresh.last_seen;
        self.availability = Availability::Online;
        self.addresses.extend(fresh.addresses);
        self.ipp_everywhere |= fresh.ipp_everywhere;
        self.airprint |= fresh.airprint;

        if upgrading {
            self.uri = fresh.uri;
            self.host = fresh.host;
            self.port = fresh.port;

            let mut merged = fresh.txt;
            merged.merge_from(&self.txt);
            self.txt = merged;
        } else {
            self.txt.merge_from(&fresh.txt);
        }

        before != (self.addresses.len(), self.txt.clone(), self.ipp_everywhere, self.airprint, self.uri.clone())
    }
}

/// `rp` carries no leading slash, so exactly one separator is supplied here.
fn build_uri(scheme: &str, host: &str, port: u16, resource_path: &str) -> Result<Url, PrinterError> {
    let authority = match host.parse::<IpAddr>() {
        Ok(IpAddr::V6(address)) => format!("[{address}]"),
        _ => host.to_owned(),
    };
    let uri = format!("{scheme}://{authority}:{port}/{resource_path}");
    Url::parse(&uri).map_err(|_| PrinterError::InvalidUri { uri })
}

// ////// //
// Errors //
// ////// //

#[derive(Debug, snafu::Snafu)]
pub enum PrinterError {
    #[snafu(display("Not an IPP service"))]
    NotAnIppService,

    #[snafu(display("Flagship naming placeholder advertised on port 0"))]
    PlaceholderService,

    #[snafu(display("Invalid printer URI {uri}"))]
    InvalidUri { uri: String },

    #[snafu(display("Unsupported URI scheme {scheme}, expected ipp or ipps"))]
    UnsupportedScheme { scheme: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_has_exactly_one_separator_before_the_resource_path() {
        let uri = build_uri("ipp", "BRW123.local", 631, "ipp/print").unwrap();
        assert_eq!(uri.as_str(), "ipp://BRW123.local:631/ipp/print");
    }

    #[test]
    fn placeholder_and_non_ipp_services_are_rejected() {
        assert!(matches!(build_uri("ipp", "", 631, "ipp/print"), Err(PrinterError::InvalidUri { .. })));
    }

    #[test]
    fn cups_style_resource_paths_keep_their_segments() {
        let uri = build_uri("ipp", "server.local", 631, "printers/Brother_HL-2030").unwrap();
        assert_eq!(uri.as_str(), "ipp://server.local:631/printers/Brother_HL-2030");
    }

    #[test]
    fn ipv6_literals_are_bracketed() {
        let uri = build_uri("ipps", "fe80::1", 631, "ipp/print").unwrap();
        assert_eq!(uri.as_str(), "ipps://[fe80::1]:631/ipp/print");
    }
}
