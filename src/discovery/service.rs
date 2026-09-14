use std::time::Duration;

use log::{info, warn};
use snafu::{ResultExt, Snafu};
use tokio::sync::mpsc;

use crate::config::models::Settings;

use super::{browser::{Browser, BrowserError, ServiceUpdate}, models::DiscoveryEvent, registry::Registry};

/// How often printers are aged towards stale and offline.
const SWEEP_INTERVAL: Duration = Duration::from_secs(15);

const CHANNEL_CAPACITY: usize = 64;

/// Drives mDNS browsing and printer bookkeeping, and reports what changed.
///
/// The registry lives inside the task, so there is no shared mutable state;
/// consumers build their own view from the event stream.
pub struct DiscoveryService;

impl DiscoveryService {
    pub fn start(settings: &Settings) -> Result<mpsc::Receiver<DiscoveryEvent>, DiscoveryError> {
        let mut registry = Registry::new(&settings.discovery, &settings.printers);
        let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);

        for printer in registry.printers() {
            if let Err(error) = sender.try_send(DiscoveryEvent::Added(printer.clone())) {
                warn!("Could not report configured printer [{}]: {error}", printer.name);
            }
        }

        if !settings.discovery.enabled {
            info!("mDNS discovery is disabled, using configured printers only");
            return Ok(receiver);
        }

        let (browser, mut updates) = Browser::start(&settings.discovery).context(BrowserSnafu)?;
        info!("Browsing for IPP printers over mDNS");

        tokio::spawn(async move {
            let mut sweep = tokio::time::interval(SWEEP_INTERVAL);

            loop {
                let events = tokio::select! {
                    _ = sweep.tick() => registry.sweep(),
                    update = updates.recv() => match update {
                        Some(ServiceUpdate::Resolved(service)) => registry.resolved(&service).into_iter().collect(),
                        Some(ServiceUpdate::Removed { fullname }) => {
                            // Reconfirm rather than remove: the sweep decides.
                            if let Some(fullname) = registry.removed(&fullname) {
                                browser.verify(&fullname);
                            }
                            continue;
                        }
                        None => break,
                    },
                };

                for event in events {
                    if sender.send(event).await.is_err() {
                        break;
                    }
                }
            }

            browser.shutdown();
        });

        Ok(receiver)
    }
}

// ////// //
// Errors //
// ////// //

#[derive(Debug, Snafu)]
pub enum DiscoveryError {
    #[snafu(display("Could not start mDNS browser: {source}"))]
    Browser { source: BrowserError },
}
