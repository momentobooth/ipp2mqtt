use std::collections::BTreeMap;

use log::{error, info};

use crate::{config::models::Settings, discovery::{models::PrinterKey, service::DiscoveryService}, ipp::{models::*, service::{IppEvent, MonitorService}}};

/// What the bridge knows about one printer. Module 4 publishes this over MQTT.
#[derive(Default)]
struct Known {
    name: String,
    capabilities: Option<PrinterCapabilities>,
    status: Option<PrinterStatus>,
    jobs: Vec<Job>,
    reachable: bool,
}

/// Runs the bridge.
pub async fn run(settings: &Settings) {
    info!("Starting ipp2mqtt v{}", env!("CARGO_PKG_VERSION"));
    info!("Running with config: {settings:#?}");

    let discovered = match DiscoveryService::start(settings) {
        Ok(events) => events,
        Err(error) => {
            error!("{error}");
            std::process::exit(1);
        }
    };

    let mut events = match MonitorService::start(settings, discovered) {
        Ok(events) => events,
        Err(error) => {
            error!("{error}");
            std::process::exit(1);
        }
    };

    let mut printers: BTreeMap<PrinterKey, Known> = BTreeMap::new();

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            event = events.recv() => match event {
                Some(event) => {
                    log_event(&event);
                    apply(&mut printers, event);
                }
                None => break,
            },
        }
    }

    info!("Shutting down with {} printer(s) known", printers.len());
}

fn apply(printers: &mut BTreeMap<PrinterKey, Known>, event: IppEvent) {
    let (key, name) = match &event {
        IppEvent::Probed { key, name, .. } | IppEvent::Status { key, name, .. }
        | IppEvent::Jobs { key, name, .. } | IppEvent::Unreachable { key, name, .. } => (key.clone(), name.clone()),
    };

    let known = printers.entry(key).or_default();
    known.name = name;

    match event {
        IppEvent::Probed { capabilities, .. } => {
            known.capabilities = Some(*capabilities);
            known.reachable = true;
        }
        IppEvent::Status { status, .. } => {
            known.status = Some(*status);
            known.reachable = true;
        }
        IppEvent::Jobs { jobs, .. } => known.jobs = jobs,
        IppEvent::Unreachable { .. } => known.reachable = false,
    }
}

fn log_event(event: &IppEvent) {
    match event {
        IppEvent::Probed { name, capabilities, .. } => {
            info!("[{name}] {} supports {}", capabilities.make_and_model.as_deref().unwrap_or("printer"), capabilities.document_formats.join(", "))
        }
        IppEvent::Status { name, status, .. } => {
            let supplies = status.supplies.iter()
                .filter_map(|s| Some(format!("{}={}%", s.colorant.as_deref().or(s.supply_type.as_deref())?, s.percent?)))
                .collect::<Vec<_>>();
            info!("[{name}] {} queued={} {}", status.state, status.queued_job_count.unwrap_or(0), supplies.join(" "))
        }
        IppEvent::Jobs { name, jobs, .. } => info!("[{name}] has {} active job(s)", jobs.len()),
        IppEvent::Unreachable { name, reason, .. } => error!("[{name}] unreachable: {reason}"),
    }
}
