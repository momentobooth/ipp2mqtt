use log::{error, info};

use crate::{config::models::Settings, ipp::{client::Transport, models::Operation, service, state}};

use super::status;

/// Makes a printer announce itself, so it can be told apart physically.
pub async fn identify(settings: &Settings, filter: Option<&str>) {
    let Some((printer, capabilities, client)) = connect(settings, filter).await else {
        return;
    };

    if !capabilities.supports_operation(Operation::IdentifyPrinter) {
        error!("[{}] does not support Identify-Printer", printer.name);
        return;
    }

    // `identify-actions-supported` says which of sound and display it can do.
    let action = capabilities.identify_actions.first().map(String::as_str).unwrap_or("sound");
    match client.identify_printer(action).await {
        Ok(()) => info!("Asked [{}] to identify itself ({action})", printer.name),
        Err(error) => error!("[{}] refused to identify: {error}", printer.name),
    }
}

/// Cancels a job and reports the state it ended in.
pub async fn cancel(settings: &Settings, job_id: i32, filter: Option<&str>) {
    let Some((printer, _, client)) = connect(settings, filter).await else {
        return;
    };

    if let Err(error) = client.cancel_job(job_id).await {
        error!("[{}] refused to cancel job #{job_id}: {error}", printer.name);
        return;
    }

    match client.get_job_attributes(job_id, state::JOB_ATTRIBUTES).await {
        Ok(response) => match state::parse_jobs(&response).first() {
            Some(job) => info!("Job #{job_id} on [{}] is now {}{}", printer.name, job.state,
                               if job.state.is_terminal() { "" } else { " (not yet terminal)" }),
            None => info!("Cancelled job #{job_id} on [{}]", printer.name),
        },
        Err(_) => info!("Cancelled job #{job_id} on [{}]", printer.name),
    }
}

/// Finds one printer and probes it, which every control command needs first.
async fn connect(settings: &Settings, filter: Option<&str>)
    -> Option<(crate::discovery::models::Printer, crate::ipp::models::PrinterCapabilities, crate::ipp::client::PrinterClient)>
{
    let printers = status::collect(settings, filter).await;
    let printer = printers.into_iter().next().or_else(|| {
        error!("No printers found{}", filter.map(|f| format!(" matching [{f}]")).unwrap_or_default());
        None
    })?;

    let (capabilities, _, _) = service::inspect(settings, &printer, false).await
        .inspect_err(|error| error!("Could not probe [{}]: {error}", printer.name))
        .ok()?;

    let transport = Transport::new(&settings.ipp)
        .inspect_err(|error| error!("Could not set up IPP transport: {error}"))
        .ok()?;
    let client = transport.printer(&printer.uri)
        .inspect_err(|error| error!("{error}"))
        .ok()?;

    Some((printer, capabilities, client))
}
