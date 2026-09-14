use std::sync::{Arc, atomic::{AtomicI32, AtomicU16, Ordering}};

use std::io::Cursor;

use futures_util::io::AllowStdIo;
use ipp::{parser::AsyncIppParser, prelude::*};
use log::{debug, warn};
use reqwest::header::{CONTENT_TYPE, HeaderValue};
use snafu::{OptionExt, ResultExt, Snafu};
use url::Url;

use crate::config::models::Ipp;

use super::{error::{self, BuildSnafu as BuildIppSnafu, IppError, ParseSnafu, TransportSnafu}, models::Operation};

const IPP_CONTENT_TYPE: HeaderValue = HeaderValue::from_static("application/ipp");

/// IPP Everywhere requires IPP/2.0. A printer that rejects it is latched down to
/// 1.1 for the rest of the session, the way the CUPS backend does.
const IPP_VERSION: u16 = 0x0200;
const IPP_VERSION_FALLBACK: u16 = 0x0101;

/// Shared HTTP clients.
///
/// Status polling and job submission want very different deadlines, so they get
/// a client each. Both pool connections, which is the point: a TLS handshake per
/// poll is the most expensive thing we could ask a printer to do.
pub struct Transport {
    poll: reqwest::Client,
    jobs: reqwest::Client,
    user_name: Arc<str>,
}

impl Transport {
    pub fn new(settings: &Ipp) -> Result<Self, TransportError> {
        let build = |timeout: std::time::Duration| {
            reqwest::Client::builder()
                .connect_timeout(settings.connect_timeout)
                .timeout(timeout)
                // Printers ship self-signed certificates, so verification is off
                // by default; see `ignoretlserrors`.
                .danger_accept_invalid_certs(settings.ignore_tls_errors)
                .danger_accept_invalid_hostnames(settings.ignore_tls_errors)
                .pool_max_idle_per_host(2)
                .user_agent(concat!("ipp2mqtt/", env!("CARGO_PKG_VERSION"), " IPP/2.0"))
                .build()
        };

        Ok(Transport {
            poll: build(settings.poll_timeout).context(ClientSnafu)?,
            jobs: build(settings.job_timeout).context(ClientSnafu)?,
            user_name: Arc::from(settings.user_name.as_str()),
        })
    }

    pub fn printer(&self, uri: &Url) -> Result<PrinterClient, TransportError> {
        Ok(PrinterClient {
            uri: uri.clone(),
            http_uri: to_http(uri).context(UriSnafu { uri: uri.to_string() })?,
            poll: self.poll.clone(),
            jobs: self.jobs.clone(),
            version: Arc::new(AtomicU16::new(IPP_VERSION)),
            request_id: Arc::new(AtomicI32::new(1)),
            user_name: self.user_name.clone(),
        })
    }
}

/// One printer's protocol state.
#[derive(Debug, Clone)]
pub struct PrinterClient {
    uri: Url,
    http_uri: Url,
    poll: reqwest::Client,
    jobs: reqwest::Client,
    version: Arc<AtomicU16>,
    request_id: Arc<AtomicI32>,
    user_name: Arc<str>,
}

impl PrinterClient {
    /// Builds a request carrying the operation attributes every operation needs.
    fn request(&self, operation: Operation) -> Result<IppRequestResponse, IppError> {
        let uri: Uri = self.uri.as_str().parse().map_err(|_| IppError::Http { uri: self.uri.to_string(), status: 0 })?;
        let version = IppVersion(self.version.load(Ordering::Relaxed));

        // The crate's constructor only takes its own Operation enum, which is
        // missing several codes real printers advertise, so the header carries
        // the raw code.
        let mut request = IppRequestResponse::new(version, ipp::model::Operation::GetPrinterAttributes, Some(uri))
            .context(BuildIppSnafu)?;
        request.header_mut().operation_or_status = operation as i32 as i16;
        request.header_mut().request_id = self.request_id.fetch_add(1, Ordering::Relaxed);
        request.attributes_mut().add(
            DelimiterTag::OperationAttributes,
            IppAttribute::with_name(IppAttribute::REQUESTING_USER_NAME, IppValue::NameWithoutLanguage(self.user_name.as_ref().try_into().context(BuildIppSnafu)?)).context(BuildIppSnafu)?,
        );

        Ok(request)
    }

    pub async fn get_printer_attributes(&self, requested: &[&str]) -> Result<IppRequestResponse, IppError> {
        let mut request = self.request(Operation::GetPrinterAttributes)?;
        add_requested_attributes(&mut request, requested)?;
        self.send(&self.poll, request, "Get-Printer-Attributes", None).await
    }

    pub async fn get_jobs(&self, which: WhichJobs, requested: &[&str], limit: Option<i32>) -> Result<IppRequestResponse, IppError> {
        let mut request = self.request(Operation::GetJobs)?;
        add_requested_attributes(&mut request, requested)?;
        add_keyword(&mut request, "which-jobs", which.as_str())?;
        if let Some(limit) = limit {
            request.attributes_mut().add(
                DelimiterTag::OperationAttributes,
                IppAttribute::with_name("limit", IppValue::Integer(limit)).context(BuildIppSnafu)?,
            );
        }
        self.send(&self.poll, request, "Get-Jobs", None).await
    }

    pub async fn get_job_attributes(&self, job_id: i32, requested: &[&str]) -> Result<IppRequestResponse, IppError> {
        let mut request = self.request(Operation::GetJobAttributes)?;
        add_job_id(&mut request, job_id)?;
        add_requested_attributes(&mut request, requested)?;
        self.send(&self.poll, request, "Get-Job-Attributes", None).await
    }

    pub async fn cancel_job(&self, job_id: i32) -> Result<(), IppError> {
        let mut request = self.request(Operation::CancelJob)?;
        add_job_id(&mut request, job_id)?;
        self.send(&self.poll, request, "Cancel-Job", None).await.map(|_| ())
    }

    /// Makes the printer announce itself. Optional in IPP Everywhere, so an
    /// unsupported answer is reported rather than treated as a fault.
    pub async fn identify_printer(&self, action: &str) -> Result<(), IppError> {
        let mut request = self.request(Operation::IdentifyPrinter)?;
        add_keyword(&mut request, "identify-actions", action)?;
        self.send(&self.poll, request, "Identify-Printer", None).await.map(|_| ())
    }

    /// Submits a document in a single operation. Print-Job is required of every
    /// printer, so it needs no capability check.
    pub async fn print_job(&self, job: &PrintJob<'_>, document: Vec<u8>) -> Result<IppRequestResponse, IppError> {
        let mut request = self.request(Operation::PrintJob)?;
        self.add_job_attributes(&mut request, job)?;
        self.send(&self.jobs, request, "Print-Job", Some(document)).await
    }

    /// Starts a multi document job. Only where `operations-supported` lists it.
    pub async fn create_job(&self, job: &PrintJob<'_>) -> Result<IppRequestResponse, IppError> {
        let mut request = self.request(Operation::CreateJob)?;
        self.add_job_attributes(&mut request, job)?;
        self.send(&self.jobs, request, "Create-Job", None).await
    }

    pub async fn send_document(&self, job_id: i32, format: Option<&str>, document: Vec<u8>, last: bool) -> Result<IppRequestResponse, IppError> {
        let mut request = self.request(Operation::SendDocument)?;
        add_job_id(&mut request, job_id)?;
        if let Some(format) = format {
            add_mime(&mut request, format)?;
        }
        request.attributes_mut().add(
            DelimiterTag::OperationAttributes,
            IppAttribute::with_name("last-document", IppValue::Boolean(last)).context(BuildIppSnafu)?,
        );
        self.send(&self.jobs, request, "Send-Document", Some(document)).await
    }

    /// Asks whether a job would be accepted, without sending the document.
    pub async fn validate_job(&self, job: &PrintJob<'_>) -> Result<IppRequestResponse, IppError> {
        let mut request = self.request(Operation::ValidateJob)?;
        self.add_job_attributes(&mut request, job)?;
        self.send(&self.poll, request, "Validate-Job", None).await
    }

    fn add_job_attributes(&self, request: &mut IppRequestResponse, job: &PrintJob<'_>) -> Result<(), IppError> {
        if let Some(name) = job.name {
            request.attributes_mut().add(
                DelimiterTag::OperationAttributes,
                IppAttribute::with_name(IppAttribute::JOB_NAME, IppValue::NameWithoutLanguage(name.try_into().context(BuildIppSnafu)?)).context(BuildIppSnafu)?,
            );
        }
        if let Some(format) = job.document_format {
            add_mime(request, format)?;
        }
        for (name, value) in &job.attributes {
            request.attributes_mut().add(
                DelimiterTag::JobAttributes,
                IppAttribute::with_name(*name, value.clone()).context(BuildIppSnafu)?,
            );
        }
        Ok(())
    }

    /// Serialises, posts, parses, and checks the IPP status.
    ///
    /// A `Content-Length` is always set rather than chunking, because some
    /// firmware rejects chunked IPP requests.
    async fn send(&self, http: &reqwest::Client, request: IppRequestResponse, operation: &str, document: Option<Vec<u8>>) -> Result<IppRequestResponse, IppError> {
        let response = self.post(http, body(&request, document.as_deref())).await?;

        // The printer rejecting our version is the one failure worth retrying
        // with a different request. Some firmware signals it as a bad request.
        let status = response.header().status_code() as u16;
        let downgradeable = matches!(status, 0x0503 | 0x0400) && self.version.load(Ordering::Relaxed) != IPP_VERSION_FALLBACK;
        if downgradeable {
            warn!("Printer at {} rejected IPP/2.0 ({}), retrying as IPP/1.1", self.uri, error::status_name(status));
            self.version.store(IPP_VERSION_FALLBACK, Ordering::Relaxed);

            let mut retry = request;
            retry.header_mut().version = IppVersion(IPP_VERSION_FALLBACK);
            retry.header_mut().request_id = self.request_id.fetch_add(1, Ordering::Relaxed);
            let response = self.post(http, body(&retry, document.as_deref())).await?;
            return self.check(response, operation);
        }

        self.check(response, operation)
    }

    /// Takes an already serialised body: `IppRequestResponse` is not `Sync`, so
    /// holding one by reference across an await would make the future non-`Send`.
    async fn post(&self, http: &reqwest::Client, body: Vec<u8>) -> Result<IppRequestResponse, IppError> {
        let response = http
            .post(self.http_uri.as_str())
            .header(CONTENT_TYPE, IPP_CONTENT_TYPE)
            .body(body)
            .send()
            .await
            .context(TransportSnafu { uri: self.uri.to_string() })?;

        let status = response.status();
        if !status.is_success() {
            return Err(IppError::Http { uri: self.uri.to_string(), status: status.as_u16() });
        }

        // IPP responses are small, so the body is buffered rather than streamed.
        let body = response.bytes().await.context(TransportSnafu { uri: self.uri.to_string() })?;
        AsyncIppParser::new(AllowStdIo::new(Cursor::new(body))).parse().await.context(ParseSnafu { uri: self.uri.to_string() })
    }

    fn check(&self, response: IppRequestResponse, operation: &str) -> Result<IppRequestResponse, IppError> {
        let status = response.header().status_code() as u16;
        match error::classify(status) {
            None => {
                if status != 0 {
                    debug!("{operation} on {} returned {}", self.uri, error::status_name(status));
                }
                Ok(response)
            }
            Some(retry) => Err(IppError::Status {
                operation: operation.to_owned(),
                status,
                name: error::status_name(status).to_owned(),
                retry,
            }),
        }
    }
}

/// A job to submit.
#[derive(Debug, Default)]
pub struct PrintJob<'a> {
    pub name: Option<&'a str>,
    /// Omit to let the printer sniff the format.
    pub document_format: Option<&'a str>,
    /// Job template attributes such as `media`, `print-color-mode`, `copies`.
    pub attributes: Vec<(&'a str, IppValue)>,
}

/// `which-jobs` values every printer must support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhichJobs {
    NotCompleted,
    All,
}

impl WhichJobs {
    fn as_str(&self) -> &'static str {
        match self {
            WhichJobs::NotCompleted => "not-completed",
            WhichJobs::All => "all",
        }
    }
}

// /////// //
// Helpers //
// /////// //

/// Serialises a request, with any document appended.
fn body(request: &IppRequestResponse, document: Option<&[u8]>) -> Vec<u8> {
    let mut body = request.to_bytes().to_vec();
    if let Some(document) = document {
        body.extend_from_slice(document);
    }
    body
}

/// IPP rides on HTTP: `ipp` is `http` and `ipps` is `https`, both on the port
/// the URI carries, defaulting to 631 rather than the HTTP default.
///
/// The URL is rebuilt rather than rewritten in place: `Url::set_scheme` refuses
/// to move between a non-special scheme such as `ipp` and a special one such as
/// `http`, and reports that by doing nothing.
fn to_http(uri: &Url) -> Option<Url> {
    let scheme = match uri.scheme() {
        "ipps" | "https" => "https",
        "ipp" | "http" => "http",
        _ => return None,
    };
    let host = uri.host_str()?;
    let port = uri.port().unwrap_or(631);

    let mut rebuilt = format!("{scheme}://{host}:{port}{}", uri.path());
    if let Some(query) = uri.query() {
        rebuilt.push('?');
        rebuilt.push_str(query);
    }
    Url::parse(&rebuilt).ok()
}

fn add_requested_attributes(request: &mut IppRequestResponse, requested: &[&str]) -> Result<(), IppError> {
    if requested.is_empty() {
        return Ok(());
    }
    let values = requested.iter()
        .map(|name| Ok(IppValue::Keyword((*name).try_into().context(BuildIppSnafu)?)))
        .collect::<Result<Vec<_>, IppError>>()?;
    request.attributes_mut().add(
        DelimiterTag::OperationAttributes,
        IppAttribute::with_name(IppAttribute::REQUESTED_ATTRIBUTES, IppValue::Array(values)).context(BuildIppSnafu)?,
    );
    Ok(())
}

fn add_keyword(request: &mut IppRequestResponse, name: &str, value: &str) -> Result<(), IppError> {
    request.attributes_mut().add(
        DelimiterTag::OperationAttributes,
        IppAttribute::with_name(name, IppValue::Keyword(value.try_into().context(BuildIppSnafu)?)).context(BuildIppSnafu)?,
    );
    Ok(())
}

fn add_mime(request: &mut IppRequestResponse, format: &str) -> Result<(), IppError> {
    request.attributes_mut().add(
        DelimiterTag::OperationAttributes,
        IppAttribute::with_name(IppAttribute::DOCUMENT_FORMAT, IppValue::MimeMediaType(format.try_into().context(BuildIppSnafu)?)).context(BuildIppSnafu)?,
    );
    Ok(())
}

fn add_job_id(request: &mut IppRequestResponse, job_id: i32) -> Result<(), IppError> {
    request.attributes_mut().add(
        DelimiterTag::OperationAttributes,
        IppAttribute::with_name(IppAttribute::JOB_ID, IppValue::Integer(job_id)).context(BuildIppSnafu)?,
    );
    Ok(())
}

// ////// //
// Errors //
// ////// //

#[derive(Debug, Snafu)]
pub enum TransportError {
    #[snafu(display("Could not build HTTP client: {source}"))]
    Client { source: reqwest::Error },

    #[snafu(display("{uri} is not a usable IPP URI"))]
    Uri { uri: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipp_uris_map_onto_http_keeping_their_port() {
        let map = |uri: &str| to_http(&Url::parse(uri).unwrap()).unwrap().to_string();
        assert_eq!(map("ipp://printer.local:631/ipp/print"), "http://printer.local:631/ipp/print");
        assert_eq!(map("ipps://printer.local:631/ipp/print"), "https://printer.local:631/ipp/print");
        assert_eq!(map("ipp://printer.local:60001/ipp/print"), "http://printer.local:60001/ipp/print");
    }

    #[test]
    fn a_missing_port_defaults_to_631_rather_than_the_http_default() {
        assert_eq!(to_http(&Url::parse("ipps://printer.local/ipp/print").unwrap()).unwrap().to_string(), "https://printer.local:631/ipp/print");
    }

    #[test]
    fn ipv6_literals_keep_their_brackets() {
        assert_eq!(to_http(&Url::parse("ipp://[fe80::1]:631/ipp/print").unwrap()).unwrap().to_string(), "http://[fe80::1]:631/ipp/print");
    }

    #[test]
    fn a_non_ipp_scheme_is_rejected_rather_than_silently_passed_through() {
        assert!(to_http(&Url::parse("ftp://printer.local/x").unwrap()).is_none());
    }
}
