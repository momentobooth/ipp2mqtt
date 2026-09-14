use std::time::Duration;

use log::{error, info};

use crate::{config::models::Settings, discovery::{models::{DiscoveryEvent, Printer}, service::DiscoveryService}, ipp::{models::*, service}};

/// How long to browse before giving up on finding the requested printer.
const DISCOVERY_WINDOW: Duration = Duration::from_secs(5);

/// Probes printers over IPP and prints their capabilities and status.
pub async fn run(settings: &Settings, filter: Option<&str>, all_jobs: bool) {
    let printers = collect(settings, filter).await;
    if printers.is_empty() {
        error!("No printers found{}", filter.map(|f| format!(" matching [{f}]")).unwrap_or_default());
        return;
    }

    for printer in &printers {
        match service::inspect(settings, printer, all_jobs).await {
            Ok((capabilities, status, jobs)) => info!("{}", describe(printer, &capabilities, &status, &jobs)),
            Err(error) => error!("Could not query [{}] at {}: {error}", printer.name, printer.uri),
        }
    }
}

/// Gathers printers from configuration and, when discovery is on, from mDNS.
pub async fn collect(settings: &Settings, filter: Option<&str>) -> Vec<Printer> {
    let Ok(mut events) = DiscoveryService::start(settings) else {
        return Vec::new();
    };

    let mut printers: Vec<Printer> = Vec::new();
    let deadline = tokio::time::sleep(DISCOVERY_WINDOW);
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            _ = &mut deadline => break,
            _ = tokio::signal::ctrl_c() => break,
            event = events.recv() => match event {
                Some(DiscoveryEvent::Added(printer)) => {
                    if filter.is_none_or(|f| printer.name.to_lowercase().contains(&f.to_lowercase())) {
                        printers.push(printer);
                    }
                }
                Some(_) => {}
                None => break,
            },
        }
    }

    printers
}

fn describe(printer: &Printer, capabilities: &PrinterCapabilities, status: &PrinterStatus, jobs: &[Job]) -> String {
    let list = |items: &[String]| if items.is_empty() { "-".to_owned() } else { items.join(", ") };
    let mut out = String::new();

    out.push_str(&format!(
        "\n[{}] at {}\n  \
           model:      {}\n  \
           firmware:   {}\n  \
           uuid:       {}\n  \
           everywhere: {}   ipp versions: {}\n  \
           formats:    {}\n  \
           preferred:  {}\n  \
           media:      {}\n  \
           ready:      {}\n  \
           color:      {}\n  \
           raster:     {} @ {}\n",
        printer.name,
        printer.uri,
        capabilities.make_and_model.as_deref().unwrap_or("-"),
        capabilities.firmware_version.as_deref().unwrap_or("-"),
        capabilities.uuid.as_deref().unwrap_or("-"),
        capabilities.is_ipp_everywhere(),
        list(&capabilities.ipp_versions),
        list(&capabilities.document_formats),
        capabilities.document_format_preferred.as_deref().unwrap_or("-"),
        list(&capabilities.media_supported),
        list(&capabilities.media_ready),
        list(&capabilities.print_color_modes),
        list(&capabilities.pwg_raster_types),
        list(&capabilities.pwg_raster_resolutions),
    ));

    out.push_str(&format!(
        "\n  state:      {} ({})\n  \
           reasons:    {}\n  \
           accepting:  {}   queued: {}\n  \
           can submit: {}   will print now: {}   needs attention: {}\n",
        status.state,
        status.state_message.as_deref().unwrap_or("-"),
        match status.state_reasons.is_empty() {
            true => "none".to_owned(),
            false => status.state_reasons.iter().map(|r| format!("{} ({:?})", r.keyword, r.severity)).collect::<Vec<_>>().join(", "),
        },
        status.is_accepting_jobs.map(|v| v.to_string()).unwrap_or("unknown".to_owned()),
        status.queued_job_count.map(|v| v.to_string()).unwrap_or("-".to_owned()),
        status.can_submit,
        status.will_print_now,
        status.needs_attention,
    ));

    if let Some(severity) = status.max_severity() {
        out.push_str(&format!("  worst:      {severity:?}\n"));
    }

    if !status.supplies.is_empty() {
        out.push_str("\n  supplies:\n");
        for supply in &status.supplies {
            let label = supply.description.clone()
                .or_else(|| supply.colorant.clone())
                .or_else(|| supply.supply_type.clone())
                .unwrap_or_else(|| "supply".to_owned());
            let level = match (supply.percent, supply.some_remaining) {
                (Some(percent), _) => format!("{percent}%"),
                (None, true) => "some remaining".to_owned(),
                (None, false) => "unknown".to_owned(),
            };
            out.push_str(&format!("    {label:<28} {level:>15}{}\n", if supply.is_receptacle { "  (receptacle)" } else { "" }));
        }
    }

    if !jobs.is_empty() {
        out.push_str("\n  jobs:\n");
        for job in jobs {
            out.push_str(&format!("    #{} {:<28} {}{}\n",
                job.id,
                job.name.as_deref().unwrap_or("-"),
                job.state,
                job.impressions_completed.map(|n| format!(" ({n} done)")).unwrap_or_default()));
        }
    }

    out
}
