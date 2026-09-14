use std::time::Duration;

use log::{debug, warn};
use mdns_sd::{IfKind, ResolvedService, ServiceDaemon, ServiceEvent};
use snafu::{ResultExt, Snafu};
use tokio::sync::mpsc;

use crate::config::models::Discovery;

/// Browsed service types. Both parent types are browsed for instances, and both
/// IPP Everywhere subtypes so `sub_ty_domain` lands in the daemon cache.
const SERVICE_TYPES: &[&str] = &[
    "_ipp._tcp.local.",
    "_ipps._tcp.local.",
    "_print._sub._ipp._tcp.local.",
    "_print._sub._ipps._tcp.local.",
    "_universal._sub._ipp._tcp.local.",
];

/// How often the daemon rescans for interfaces that appear later.
const IP_CHECK_INTERVAL_SECS: u32 = 5;

/// RFC 6762 section 10.4 reconfirmation window.
const VERIFY_TIMEOUT: Duration = Duration::from_secs(10);

const CHANNEL_CAPACITY: usize = 256;

/// One service instance appearing or disappearing. Raw mDNS, no identity or
/// lifecycle logic; that belongs to the registry.
#[derive(Debug)]
pub enum ServiceUpdate {
    Resolved(Box<ResolvedService>),
    Removed { fullname: String },
}

/// Owns the mDNS daemon and fans every browse stream into one channel.
pub struct Browser {
    daemon: ServiceDaemon,
}

impl Browser {
    pub fn start(settings: &Discovery) -> Result<(Self, mpsc::Receiver<ServiceUpdate>), BrowserError> {
        let daemon = ServiceDaemon::new().context(DaemonSnafu)?;
        daemon.set_ip_check_interval(IP_CHECK_INTERVAL_SECS).context(DaemonSnafu)?;

        // An allowlist means disabling everything first; VPN and container
        // interfaces otherwise get joined and carry no printers.
        if !settings.interfaces.is_empty() {
            daemon.disable_interface(IfKind::All).context(DaemonSnafu)?;
            for name in &settings.interfaces {
                daemon.enable_interface(IfKind::Name(name.clone())).context(DaemonSnafu)?;
            }
        }
        for name in &settings.ignore_interfaces {
            daemon.disable_interface(IfKind::Name(name.clone())).context(DaemonSnafu)?;
        }

        let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
        for service_type in SERVICE_TYPES {
            let events = daemon.browse(service_type).context(BrowseSnafu { service_type: *service_type })?;
            let sender = sender.clone();
            let service_type = *service_type;

            // One task per browse keeps each receiver owned by a single task.
            tokio::spawn(async move {
                while let Ok(event) = events.recv_async().await {
                    let update = match event {
                        ServiceEvent::ServiceResolved(service) => {
                            debug!("Resolved [{}] ty={} sub={:?} via [{service_type}]", service.fullname, service.ty_domain, service.sub_ty_domain);
                            ServiceUpdate::Resolved(service)
                        }
                        ServiceEvent::ServiceRemoved(_, fullname) => ServiceUpdate::Removed { fullname },
                        ServiceEvent::ServiceFound(_, fullname) => {
                            debug!("Found [{fullname}] on [{service_type}]");
                            continue;
                        }
                        _ => continue,
                    };
                    if sender.send(update).await.is_err() {
                        break;
                    }
                }
                debug!("Browse stream for [{service_type}] ended");
            });
        }

        Ok((Self { daemon }, receiver))
    }

    /// Shortens the cached TTLs for an instance and re-queries it. Call this when
    /// a service goes away or when a connection to a cached address fails, so the
    /// cache learns about it instead of waiting out the full TTL.
    pub fn verify(&self, fullname: &str) {
        if let Err(error) = self.daemon.verify(fullname.to_owned(), VERIFY_TIMEOUT) {
            warn!("Could not reconfirm [{fullname}]: {error}");
        }
    }

    pub fn shutdown(&self) {
        if let Err(error) = self.daemon.shutdown() {
            warn!("Could not shut down mDNS daemon: {error}");
        }
    }
}

// ////// //
// Errors //
// ////// //

#[derive(Debug, Snafu)]
pub enum BrowserError {
    #[snafu(display("Could not start mDNS daemon: {source}"))]
    Daemon { source: mdns_sd::Error },

    #[snafu(display("Could not browse {service_type}: {source}"))]
    Browse { service_type: String, source: mdns_sd::Error },
}
