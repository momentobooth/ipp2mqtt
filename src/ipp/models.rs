use std::fmt::Display;

use serde::Serialize;

use super::supplies::Supply;

// /////// //
// Printer //
// /////// //

/// `printer-state`. Only 3, 4 and 5 are registered; anything else is kept as a
/// number rather than being folded into a known state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrinterState {
    Idle,
    Processing,
    Stopped,
    Unknown(i32),
}

impl PrinterState {
    pub fn from_code(code: i32) -> Self {
        match code {
            3 => PrinterState::Idle,
            4 => PrinterState::Processing,
            5 => PrinterState::Stopped,
            other => PrinterState::Unknown(other),
        }
    }
}

impl Display for PrinterState {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            PrinterState::Idle => f.write_str("idle"),
            PrinterState::Processing => f.write_str("processing"),
            PrinterState::Stopped => f.write_str("stopped"),
            PrinterState::Unknown(code) => write!(f, "unknown({code})"),
        }
    }
}

/// Severity of a state reason, from the keyword suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Report,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StateReason {
    /// The keyword with its severity suffix removed.
    pub keyword: String,
    pub severity: Severity,
}

impl StateReason {
    /// Splits the severity suffix off a reason keyword.
    ///
    /// An unsuffixed keyword takes its severity from the printer state: an error
    /// when stopped, a warning otherwise.
    pub fn parse(keyword: &str, state: PrinterState) -> Option<StateReason> {
        if keyword.eq_ignore_ascii_case("none") {
            return None;
        }

        let (keyword, severity) = match keyword {
            _ if keyword.ends_with("-error") => (keyword.trim_end_matches("-error"), Severity::Error),
            _ if keyword.ends_with("-warning") => (keyword.trim_end_matches("-warning"), Severity::Warning),
            _ if keyword.ends_with("-report") => (keyword.trim_end_matches("-report"), Severity::Report),
            _ if state == PrinterState::Stopped => (keyword, Severity::Error),
            _ => (keyword, Severity::Warning),
        };

        Some(StateReason { keyword: keyword.to_owned(), severity })
    }
}

/// What a printer is doing right now.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrinterStatus {
    pub state: PrinterState,
    pub state_reasons: Vec<StateReason>,
    pub state_message: Option<String>,
    pub is_accepting_jobs: Option<bool>,
    pub queued_job_count: Option<i32>,
    pub up_time: Option<i32>,
    /// Seconds the printer has been in its current state, where it reports both
    /// `printer-up-time` and `printer-state-change-time`.
    pub seconds_in_state: Option<i32>,
    pub config_change_time: Option<i32>,
    pub supplies: Vec<Supply>,
    /// Whether a job submitted now would be accepted. Independent of the state:
    /// a stopped printer commonly still queues work.
    pub can_submit: bool,
    /// Whether a job submitted now would actually print without intervention.
    pub will_print_now: bool,
    pub needs_attention: bool,
}

impl PrinterStatus {
    pub fn max_severity(&self) -> Option<Severity> {
        self.state_reasons.iter().map(|reason| reason.severity).max()
    }
}

/// What a printer can do. Fetched once and refreshed when its configuration
/// changes, because it is large and almost static.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PrinterCapabilities {
    pub name: Option<String>,
    pub info: Option<String>,
    pub make_and_model: Option<String>,
    pub location: Option<String>,
    pub uuid: Option<String>,
    pub device_id: Option<String>,
    pub more_info: Option<String>,
    pub firmware_version: Option<String>,
    pub ipp_versions: Vec<String>,
    pub ipp_features: Vec<String>,
    /// Kept as raw operation codes; the registry has more of them than any enum.
    pub operations: Vec<i32>,
    pub document_formats: Vec<String>,
    pub document_format_default: Option<String>,
    pub document_format_preferred: Option<String>,
    pub urf: Vec<String>,
    pub pwg_raster_types: Vec<String>,
    pub pwg_raster_resolutions: Vec<String>,
    pub pwg_raster_sheet_back: Option<String>,
    pub media_supported: Vec<String>,
    pub media_default: Option<String>,
    pub media_ready: Vec<String>,
    pub print_color_modes: Vec<String>,
    pub print_qualities: Vec<i32>,
    pub sides: Vec<String>,
    pub resolutions: Vec<String>,
    pub copies_max: Option<i32>,
    pub identify_actions: Vec<String>,
    pub config_change_time: Option<i32>,
}

impl PrinterCapabilities {
    pub fn supports_operation(&self, operation: Operation) -> bool {
        self.operations.contains(&(operation as i32))
    }

    pub fn supports_format(&self, format: &str) -> bool {
        self.document_formats.iter().any(|f| f.eq_ignore_ascii_case(format))
    }

    /// Whether the printer claims IPP Everywhere conformance. This is the
    /// authoritative flag; the mDNS subtype is only a discovery-time proxy.
    pub fn is_ipp_everywhere(&self) -> bool {
        self.ipp_features.iter().any(|f| f.eq_ignore_ascii_case("ipp-everywhere"))
    }
}

/// Operation codes used by this client. The `ipp` crate's own enum is missing
/// several that real printers advertise, so these are defined here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub enum Operation {
    PrintJob = 0x0002,
    ValidateJob = 0x0004,
    CreateJob = 0x0005,
    SendDocument = 0x0006,
    CancelJob = 0x0008,
    GetJobAttributes = 0x0009,
    GetJobs = 0x000A,
    GetPrinterAttributes = 0x000B,
    IdentifyPrinter = 0x003C,
}

// //// //
// Jobs //
// //// //

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Pending,
    PendingHeld,
    Processing,
    ProcessingStopped,
    Canceled,
    Aborted,
    Completed,
    Unknown(i32),
}

impl JobState {
    pub fn from_code(code: i32) -> Self {
        match code {
            3 => JobState::Pending,
            4 => JobState::PendingHeld,
            5 => JobState::Processing,
            6 => JobState::ProcessingStopped,
            7 => JobState::Canceled,
            8 => JobState::Aborted,
            9 => JobState::Completed,
            other => JobState::Unknown(other),
        }
    }

    /// Whether the job will never change state again.
    pub fn is_terminal(&self) -> bool {
        matches!(self, JobState::Canceled | JobState::Aborted | JobState::Completed)
    }
}

impl Display for JobState {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            JobState::Pending => f.write_str("pending"),
            JobState::PendingHeld => f.write_str("pending-held"),
            JobState::Processing => f.write_str("processing"),
            JobState::ProcessingStopped => f.write_str("processing-stopped"),
            JobState::Canceled => f.write_str("canceled"),
            JobState::Aborted => f.write_str("aborted"),
            JobState::Completed => f.write_str("completed"),
            JobState::Unknown(code) => write!(f, "unknown({code})"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Job {
    pub id: i32,
    pub uuid: Option<String>,
    pub name: Option<String>,
    pub state: JobState,
    pub state_reasons: Vec<String>,
    pub state_message: Option<String>,
    pub originating_user: Option<String>,
    pub impressions: Option<i32>,
    pub impressions_completed: Option<i32>,
    pub media_sheets_completed: Option<i32>,
    pub time_at_creation: Option<i32>,
    pub time_at_completed: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_suffixes_win_over_the_printer_state() {
        let reason = StateReason::parse("media-empty-error", PrinterState::Idle).unwrap();
        assert_eq!(reason, StateReason { keyword: "media-empty".to_owned(), severity: Severity::Error });

        let reason = StateReason::parse("toner-low-report", PrinterState::Stopped).unwrap();
        assert_eq!(reason.severity, Severity::Report);
    }

    #[test]
    fn a_bare_keyword_takes_its_severity_from_the_state() {
        assert_eq!(StateReason::parse("media-jam", PrinterState::Stopped).unwrap().severity, Severity::Error);
        assert_eq!(StateReason::parse("media-jam", PrinterState::Idle).unwrap().severity, Severity::Warning);
    }

    #[test]
    fn none_is_not_a_reason() {
        assert!(StateReason::parse("none", PrinterState::Idle).is_none());
        assert!(StateReason::parse("NONE", PrinterState::Stopped).is_none());
    }

    #[test]
    fn unregistered_states_keep_their_number() {
        assert_eq!(PrinterState::from_code(9), PrinterState::Unknown(9));
        assert_eq!(JobState::from_code(42), JobState::Unknown(42));
        assert!(!JobState::Unknown(42).is_terminal());
    }

    #[test]
    fn terminal_job_states() {
        assert!(JobState::Completed.is_terminal());
        assert!(JobState::Aborted.is_terminal());
        assert!(JobState::Canceled.is_terminal());
        assert!(!JobState::Processing.is_terminal());
        assert!(!JobState::PendingHeld.is_terminal());
    }
}
