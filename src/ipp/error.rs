use snafu::Snafu;

/// What a caller should do about a failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retry {
    /// Transient. Retry the unmodified request after seconds, with backoff.
    Soon,
    /// The printer cannot serve this now but should later. Re-probe its state
    /// rather than retrying on a blind timer.
    Later,
    /// The request as formed will never succeed. Do not retry it unmodified.
    Never,
    /// Credentials are needed or wrong. Retry only once they change.
    Auth,
    /// The printer does not implement this operation. Fall back to another
    /// approach and stop asking.
    Unsupported,
}

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum IppError {
    #[snafu(display("Could not reach printer at {uri}: {source}"))]
    Transport { uri: String, source: reqwest::Error },

    #[snafu(display("Printer at {uri} answered HTTP {status}"))]
    Http { uri: String, status: u16 },

    #[snafu(display("Could not build IPP request: {source}"))]
    Build { source: ipp::parser::IppParseError },

    #[snafu(display("Could not parse IPP response from {uri}: {source}"))]
    Parse { uri: String, source: ipp::parser::IppParseError },

    #[snafu(display("{operation} failed with IPP status {status:#06x} ({name})"))]
    Status { operation: String, status: u16, name: String, retry: Retry },
}

impl IppError {
    pub fn retry(&self) -> Retry {
        match self {
            IppError::Status { retry, .. } => *retry,
            // A connection that never opened or died mid-flight is worth another
            // go; a malformed response or an unbuildable request is not.
            IppError::Transport { source, .. } if source.is_connect() || source.is_timeout() || source.is_request() => Retry::Soon,
            IppError::Transport { .. } => Retry::Later,
            IppError::Http { status, .. } => match status {
                401 | 403 => Retry::Auth,
                404 | 405 | 501 => Retry::Never,
                408 | 429 | 500 | 502 | 503 | 504 => Retry::Soon,
                _ => Retry::Never,
            },
            IppError::Build { .. } | IppError::Parse { .. } => Retry::Never,
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(self.retry(), Retry::Soon | Retry::Later)
    }
}

/// Classifies an IPP status code.
///
/// Ranges are honoured rather than only the registered codes, so a vendor code
/// lands in the right class instead of being treated as a hard failure.
pub fn classify(status: u16) -> Option<Retry> {
    match status {
        // Successful, including the vendor range.
        0x0000..=0x00ff => None,
        // Informational; no IPP semantics are defined, treat as success.
        0x0100..=0x01ff => None,

        0x0401..=0x0403 => Some(Retry::Auth),
        // Busy, timed out, or a conflict that may clear on its own.
        0x0405 | 0x0507 | 0x0505 | 0x0502 | 0x0500 | 0x0504 => Some(Retry::Soon),
        // Administratively or physically unable right now.
        0x0404 | 0x0506 => Some(Retry::Later),
        // The printer does not implement the operation.
        0x0501 => Some(Retry::Unsupported),

        _ => Some(Retry::Never),
    }
}

/// Human readable name for a status code, for logs and MQTT payloads.
pub fn status_name(status: u16) -> &'static str {
    match status {
        0x0000 => "successful-ok",
        0x0001 => "successful-ok-ignored-or-substituted-attributes",
        0x0002 => "successful-ok-conflicting-attributes",
        0x0400 => "client-error-bad-request",
        0x0401 => "client-error-forbidden",
        0x0402 => "client-error-not-authenticated",
        0x0403 => "client-error-not-authorized",
        0x0404 => "client-error-not-possible",
        0x0405 => "client-error-timeout",
        0x0406 => "client-error-not-found",
        0x0407 => "client-error-gone",
        0x0408 => "client-error-request-entity-too-large",
        0x0409 => "client-error-request-value-too-long",
        0x040a => "client-error-document-format-not-supported",
        0x040b => "client-error-attributes-or-values-not-supported",
        0x040c => "client-error-uri-scheme-not-supported",
        0x040d => "client-error-charset-not-supported",
        0x040e => "client-error-conflicting-attributes",
        0x040f => "client-error-compression-not-supported",
        0x0410 => "client-error-compression-error",
        0x0411 => "client-error-document-format-error",
        0x0412 => "client-error-document-access-error",
        0x0500 => "server-error-internal-error",
        0x0501 => "server-error-operation-not-supported",
        0x0502 => "server-error-service-unavailable",
        0x0503 => "server-error-version-not-supported",
        0x0504 => "server-error-device-error",
        0x0505 => "server-error-temporary-error",
        0x0506 => "server-error-not-accepting-jobs",
        0x0507 => "server-error-busy",
        0x0508 => "server-error-job-canceled",
        0x0509 => "server-error-multiple-document-jobs-not-supported",
        0x050a => "server-error-printer-is-deactivated",
        0x050b => "server-error-too-many-jobs",
        0x050c => "server-error-too-many-documents",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_success_code_including_the_vendor_range_is_not_an_error() {
        for status in [0x0000, 0x0001, 0x0002, 0x0007, 0x0080, 0x00ff, 0x0100] {
            assert_eq!(classify(status), None, "{status:#06x} should be a success");
        }
    }

    #[test]
    fn transient_failures_retry_soon() {
        for status in [0x0507, 0x0505, 0x0502, 0x0500, 0x0504, 0x0405] {
            assert_eq!(classify(status), Some(Retry::Soon), "{status:#06x} should retry soon");
        }
    }

    #[test]
    fn a_printer_refusing_work_retries_later() {
        assert_eq!(classify(0x0506), Some(Retry::Later));
        assert_eq!(classify(0x0404), Some(Retry::Later));
    }

    #[test]
    fn a_bad_document_is_never_retried() {
        assert_eq!(classify(0x040a), Some(Retry::Never));
        assert_eq!(classify(0x0411), Some(Retry::Never));
        assert_eq!(classify(0x040b), Some(Retry::Never));
    }

    #[test]
    fn an_unimplemented_operation_is_distinct_from_a_bad_request() {
        assert_eq!(classify(0x0501), Some(Retry::Unsupported));
        assert_eq!(classify(0x0400), Some(Retry::Never));
    }

    #[test]
    fn authentication_failures_are_their_own_class() {
        for status in [0x0401, 0x0402, 0x0403] {
            assert_eq!(classify(status), Some(Retry::Auth));
        }
    }
}
