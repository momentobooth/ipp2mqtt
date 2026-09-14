use std::collections::BTreeMap;

use log::{error, info};

use crate::{config::models::Settings, discovery::{models::*, service::DiscoveryService}};

/// Browses for printers and reports what is found, until interrupted.
pub async fn run(settings: &Settings) {
    let mut events = match DiscoveryService::start(settings) {
        Ok(events) => events,
        Err(error) => {
            error!("{error}");
            std::process::exit(1);
        }
    };

    // Consumers keep their own view of the printers; the registry itself stays
    // inside the discovery task.
    let mut printers: BTreeMap<PrinterKey, Printer> = BTreeMap::new();

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            event = events.recv() => match event {
                Some(event) => {
                    log_event(&event);
                    let printer = match event {
                        DiscoveryEvent::Added(printer) | DiscoveryEvent::Updated(printer) => printer,
                        DiscoveryEvent::AvailabilityChanged { printer, .. } => printer,
                    };
                    printers.insert(printer.key.clone(), printer);
                }
                None => break,
            },
        }
    }

    info!("Discovered {} printer(s)", printers.len());
    for printer in printers.values() {
        info!("[{}] is {}\n{}", printer.name, printer.availability, describe(printer));
    }
}

pub fn log_event(event: &DiscoveryEvent) {
    match event {
        DiscoveryEvent::Added(printer) => info!("Found printer [{}] at {}", printer.name, printer.uri),
        DiscoveryEvent::Updated(printer) => info!("Updated printer [{}] at {}", printer.name, printer.uri),
        DiscoveryEvent::AvailabilityChanged { printer, previous } => {
            info!("Printer [{}] went from {previous} to {}", printer.name, printer.availability)
        }
    }
}

fn describe(printer: &Printer) -> String {
    let txt = &printer.txt;
    let list = |items: Vec<String>| if items.is_empty() { "-".to_owned() } else { items.join(", ") };
    let addresses = printer.addresses.iter().map(|a| a.to_string()).collect::<Vec<_>>();

    format!(
        "  uri:        {}\n  \
           key:        {}\n  \
           model:      {}\n  \
           location:   {}\n  \
           admin:      {}\n  \
           addresses:  {}\n  \
           formats:    {}\n  \
           urf:        {}\n  \
           media:      {}\n  \
           auth:       {}  tls: {}\n  \
           everywhere: {}  airprint: {}  color: {}  duplex: {}  paper max: {}",
        printer.uri,
        printer.key,
        txt.make_and_model().unwrap_or("-"),
        txt.location().unwrap_or("-"),
        txt.admin_url().unwrap_or("-"),
        list(addresses),
        list(txt.document_formats()),
        list(txt.urf()),
        list(txt.media_kinds()),
        txt.authentication().unwrap_or("-"),
        txt.tls_version().unwrap_or("-"),
        printer.ipp_everywhere,
        printer.airprint,
        tri_state(txt.color()),
        tri_state(txt.duplex()),
        txt.paper_max().unwrap_or("-"),
    )
}

fn tri_state(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "unknown",
    }
}
