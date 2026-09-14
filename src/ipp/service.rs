use std::{collections::HashMap, time::Duration};

use exponential_backoff::Backoff;
use log::{debug, info, warn};
use snafu::{OptionExt, ResultExt, Snafu};
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{config::models::Settings, discovery::models::{DiscoveryEvent, Printer, PrinterKey}};

use super::{capabilities, client::{PrinterClient, Transport, TransportError, WhichJobs}, error::IppError, models::*, state};

const CHANNEL_CAPACITY: usize = 64;

/// Bounds on how far a failing printer backs off before trying again.
const RETRY_MIN: Duration = Duration::from_secs(2);
const RETRY_MAX: Duration = Duration::from_secs(60);
const RETRY_ATTEMPTS: u32 = 8;

#[derive(Debug, Clone)]
pub enum IppEvent {
    Probed { key: PrinterKey, name: String, capabilities: Box<PrinterCapabilities> },
    Status { key: PrinterKey, name: String, status: Box<PrinterStatus> },
    Jobs { key: PrinterKey, name: String, jobs: Vec<Job> },
    Unreachable { key: PrinterKey, name: String, reason: String },
}

/// Polls every known printer and reports what changed.
///
/// One task per printer, so a slow or dead printer never delays the others, and
/// requests to one printer stay serialised.
pub struct MonitorService;

impl MonitorService {
    pub fn start(settings: &Settings, mut discovered: mpsc::Receiver<DiscoveryEvent>) -> Result<mpsc::Receiver<IppEvent>, MonitorError> {
        let transport = Transport::new(&settings.ipp).context(TransportSnafu)?;
        let (sender, receiver) = mpsc::channel(CHANNEL_CAPACITY);
        let poll_interval = settings.ipp.poll_interval;
        let supply_interval = settings.ipp.supply_interval;

        tokio::spawn(async move {
            let mut monitors: HashMap<PrinterKey, JoinHandle<()>> = HashMap::new();

            while let Some(event) = discovered.recv().await {
                let printer = match event {
                    DiscoveryEvent::Added(printer) | DiscoveryEvent::Updated(printer) => printer,
                    DiscoveryEvent::AvailabilityChanged { printer, .. } => printer,
                };

                // A printer whose URI changed needs its monitor rebuilt, so the
                // old task is replaced rather than left pointing at a stale host.
                if let Some(existing) = monitors.remove(&printer.key) {
                    existing.abort();
                }

                let client = match transport.printer(&printer.uri) {
                    Ok(client) => client,
                    Err(error) => {
                        warn!("Skipping [{}]: {error}", printer.name);
                        continue;
                    }
                };
                let sender = sender.clone();
                monitors.insert(
                    printer.key.clone(),
                    tokio::spawn(monitor_printer(printer, client, sender, poll_interval, supply_interval)),
                );
            }

            for monitor in monitors.values() {
                monitor.abort();
            }
        });

        Ok(receiver)
    }
}

/// Probes a printer once, then polls it until the task is dropped.
async fn monitor_printer(
    printer: Printer,
    client: PrinterClient,
    events: mpsc::Sender<IppEvent>,
    poll_interval: Duration,
    supply_interval: Duration,
) {
    let key = printer.key.clone();
    let name = printer.name.clone();
    let backoff = Backoff::new(RETRY_ATTEMPTS, RETRY_MIN, RETRY_MAX);

    let Some(mut capabilities) = probe(&client, &name, &backoff).await else {
        return;
    };
    let supports_jobs = capabilities.supports_operation(Operation::GetJobs);
    info!("Probed [{name}]: {} ({})", capabilities.make_and_model.as_deref().unwrap_or("unknown model"),
          if capabilities.is_ipp_everywhere() { "IPP Everywhere" } else { "IPP" });

    if events.send(IppEvent::Probed { key: key.clone(), name: name.clone(), capabilities: Box::new(capabilities.clone()) }).await.is_err() {
        return;
    }

    let mut poll = tokio::time::interval(poll_interval);
    let mut supplies = tokio::time::interval(supply_interval);
    let mut failures = 0u32;

    loop {
        tokio::select! {
            _ = supplies.tick() => {
                if let Ok(response) = client.get_printer_attributes(state::SUPPLY_ATTRIBUTES).await {
                    debug!("Refreshed supplies for [{name}]");
                    let _ = response;
                }
            }
            _ = poll.tick() => {
                // Supplies ride along with the status request so the two views
                // are consistent and cost one round trip.
                let requested: Vec<&str> = state::STATUS_ATTRIBUTES.iter().chain(state::SUPPLY_ATTRIBUTES).copied().collect();
                match client.get_printer_attributes(&requested).await {
                    Ok(response) => {
                        failures = 0;
                        let Some(status) = state::parse_status(&response) else {
                            warn!("Printer [{name}] answered without a printer attributes group");
                            continue;
                        };

                        // The printer telling us its configuration changed is the
                        // only reason to re-read the large capability set.
                        if status.config_change_time.is_some() && status.config_change_time != capabilities.config_change_time {
                            info!("Configuration of [{name}] changed, re-probing");
                            if let Some(fresh) = probe(&client, &name, &backoff).await {
                                capabilities = fresh;
                                let _ = events.send(IppEvent::Probed { key: key.clone(), name: name.clone(), capabilities: Box::new(capabilities.clone()) }).await;
                            }
                        }

                        let has_jobs = status.queued_job_count.unwrap_or(0) > 0;
                        if events.send(IppEvent::Status { key: key.clone(), name: name.clone(), status: Box::new(status) }).await.is_err() {
                            return;
                        }

                        // Only ask for jobs when the printer says it has some.
                        if supports_jobs && has_jobs {
                            match client.get_jobs(WhichJobs::NotCompleted, state::JOB_ATTRIBUTES, Some(50)).await {
                                Ok(response) => {
                                    let jobs = state::parse_jobs(&response);
                                    let _ = events.send(IppEvent::Jobs { key: key.clone(), name: name.clone(), jobs }).await;
                                }
                                Err(error) => debug!("Could not list jobs on [{name}]: {error}"),
                            }
                        }
                    }
                    Err(error) => {
                        failures += 1;
                        // One timeout is normal for a printer waking from sleep;
                        // report only once it is repeatable.
                        if failures == 3 {
                            warn!("Printer [{name}] is not answering: {error}");
                            let _ = events.send(IppEvent::Unreachable { key: key.clone(), name: name.clone(), reason: error.to_string() }).await;
                        }
                    }
                }
            }
        }
    }
}

async fn probe(client: &PrinterClient, name: &str, backoff: &Backoff) -> Option<PrinterCapabilities> {
    for delay in backoff {
        match client.get_printer_attributes(capabilities::CAPABILITY_ATTRIBUTES).await {
            Ok(response) => match capabilities::parse(&response) {
                Some(capabilities) => return Some(capabilities),
                None => warn!("Printer [{name}] answered a probe without a printer attributes group"),
            },
            Err(error) if !error.is_retryable() => {
                warn!("Cannot probe [{name}]: {error}");
                return None;
            }
            Err(error) => debug!("Probe of [{name}] failed, retrying: {error}"),
        }

        match delay {
            Some(delay) => tokio::time::sleep(delay).await,
            None => break,
        }
    }

    warn!("Giving up probing [{name}]");
    None
}

/// Runs a single probe and status poll, for the `status` command.
pub async fn inspect(settings: &Settings, printer: &Printer, all_jobs: bool) -> Result<(PrinterCapabilities, PrinterStatus, Vec<Job>), MonitorError> {
    let transport = Transport::new(&settings.ipp).context(TransportSnafu)?;
    let client = transport.printer(&printer.uri).context(TransportSnafu)?;

    let response = client.get_printer_attributes(capabilities::CAPABILITY_ATTRIBUTES).await.context(ProbeSnafu)?;
    let capabilities = capabilities::parse(&response).context(NoAttributesSnafu)?;

    let requested: Vec<&str> = state::STATUS_ATTRIBUTES.iter().chain(state::SUPPLY_ATTRIBUTES).copied().collect();
    let response = client.get_printer_attributes(&requested).await.context(ProbeSnafu)?;
    let status = state::parse_status(&response).context(NoAttributesSnafu)?;

    let jobs = match capabilities.supports_operation(Operation::GetJobs) {
        true => client.get_jobs(if all_jobs { WhichJobs::All } else { WhichJobs::NotCompleted }, state::JOB_ATTRIBUTES, Some(50)).await
            .map(|response| state::parse_jobs(&response))
            .unwrap_or_default(),
        false => Vec::new(),
    };

    Ok((capabilities, status, jobs))
}

// ////// //
// Errors //
// ////// //

#[derive(Debug, Snafu)]
pub enum MonitorError {
    #[snafu(display("Could not set up IPP transport: {source}"))]
    Transport { source: TransportError },

    #[snafu(display("{source}"))]
    Probe { source: IppError },

    #[snafu(display("Printer answered without a printer attributes group"))]
    NoAttributes,
}
