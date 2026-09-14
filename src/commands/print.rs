use log::{error, info};

use crate::{config::models::Settings, ipp::{client::{PrintJob, Transport}, models::Operation, service, state}};

use super::status;

/// Sends one or more documents to a printer.
pub async fn run(settings: &Settings, paths: &[String], filter: Option<&str>, format: Option<&str>) {
    let mut documents = Vec::new();
    for path in paths {
        match tokio::fs::read(path).await {
            Ok(document) => documents.push(document),
            Err(error) => {
                error!("Could not read {path}: {error}");
                return;
            }
        }
    }

    let printers = status::collect(settings, filter).await;
    let Some(printer) = printers.first() else {
        error!("No printers found{}", filter.map(|f| format!(" matching [{f}]")).unwrap_or_default());
        return;
    };

    let Ok((capabilities, _, _)) = service::inspect(settings, printer, false).await else {
        error!("Could not probe [{}]", printer.name);
        return;
    };

    // Without an explicit format the printer is left to sniff the document,
    // which is what `application/octet-stream` means.
    if let Some(format) = format
        && !capabilities.supports_format(format)
    {
        error!("[{}] does not accept {format}; it accepts {}", printer.name, capabilities.document_formats.join(", "));
        return;
    }

    let name = std::path::Path::new(&paths[0]).file_name().and_then(|n| n.to_str()).unwrap_or("ipp2mqtt");
    let job = PrintJob { name: Some(name), document_format: format, attributes: Vec::new() };

    let Ok(transport) = Transport::new(&settings.ipp) else {
        error!("Could not set up IPP transport");
        return;
    };
    let client = match transport.printer(&printer.uri) {
        Ok(client) => client,
        Err(error) => {
            error!("{error}");
            return;
        }
    };

    // Asking first costs one round trip and turns a wasted upload into a clear
    // error, but plenty of printers implement it loosely, so a failure here is
    // reported and not treated as fatal.
    if capabilities.supports_operation(Operation::ValidateJob)
        && let Err(error) = client.validate_job(&job).await
    {
        error!("[{}] rejected the job at validation: {error}", printer.name);
        return;
    }

    // PWG recommends Print-Job for a single document; Create-Job plus
    // Send-Document exists for the multi document case and is not universally
    // supported, so it is only used when it is actually needed.
    if documents.len() == 1 {
        let document = documents.remove(0);
        match client.print_job(&job, document).await {
            Ok(response) => report(&printer.name, &response),
            Err(error) => error!("[{}] rejected the job: {error}", printer.name),
        }
        return;
    }

    if !capabilities.supports_operation(Operation::CreateJob) || !capabilities.supports_operation(Operation::SendDocument) {
        error!("[{}] cannot take multi document jobs; send the files one at a time", printer.name);
        return;
    }

    let response = match client.create_job(&job).await {
        Ok(response) => response,
        Err(error) => {
            error!("[{}] rejected the job: {error}", printer.name);
            return;
        }
    };
    let Some(created) = state::parse_submitted_job(&response) else {
        error!("[{}] accepted the job but reported no job id", printer.name);
        return;
    };

    let last = documents.len() - 1;
    for (index, document) in documents.into_iter().enumerate() {
        if let Err(error) = client.send_document(created.id, format, document, index == last).await {
            error!("[{}] rejected document {} of job #{}: {error}", printer.name, index + 1, created.id);
            return;
        }
    }
    info!("[{}] accepted job #{} with {} documents", printer.name, created.id, last + 1);
}

fn report(printer: &str, response: &ipp::request::IppRequestResponse) {
    match state::parse_submitted_job(response) {
        Some(job) => info!("[{printer}] accepted job #{} ({})", job.id, job.state),
        None => info!("[{printer}] accepted the job but reported no job id"),
    }
}
