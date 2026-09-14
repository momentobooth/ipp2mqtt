use ipp::{model::DelimiterTag, request::IppRequestResponse};

use super::{attrs::Attrs, models::*, supplies::Supply};

/// Polled every cycle. Kept small on purpose: these are the values that change.
pub const STATUS_ATTRIBUTES: &[&str] = &[
    "printer-state",
    "printer-state-reasons",
    "printer-state-message",
    "printer-is-accepting-jobs",
    "queued-job-count",
    "printer-up-time",
    "printer-state-change-time",
    "printer-config-change-time",
];

/// Polled less often: supplies and loaded media move slowly and cost more.
pub const SUPPLY_ATTRIBUTES: &[&str] = &[
    "printer-supply",
    "printer-supply-description",
    "media-ready",
];

/// Attributes read for one job.
pub const JOB_ATTRIBUTES: &[&str] = &[
    "job-id",
    "job-uuid",
    "job-name",
    "job-state",
    "job-state-reasons",
    "job-state-message",
    "job-originating-user-name",
    "job-impressions",
    "job-impressions-completed",
    "job-media-sheets-completed",
    "time-at-creation",
    "time-at-completed",
];

pub fn parse_status(response: &IppRequestResponse) -> Option<PrinterStatus> {
    let attrs = Attrs::of(response, DelimiterTag::PrinterAttributes)?;

    let state = attrs.enumeration("printer-state").map(PrinterState::from_code).unwrap_or(PrinterState::Unknown(0));
    let state_reasons: Vec<_> = attrs.texts("printer-state-reasons")
        .iter()
        .filter_map(|keyword| StateReason::parse(keyword, state))
        .collect();

    let up_time = attrs.integer("printer-up-time");
    let is_accepting_jobs = attrs.boolean("printer-is-accepting-jobs");
    let worst = state_reasons.iter().map(|reason| reason.severity).max();

    // Three signals rather than one flag: `printer-is-accepting-jobs` governs
    // future jobs and is explicitly independent of the state, so a stopped
    // printer commonly still queues work.
    let can_submit = is_accepting_jobs.unwrap_or(true);
    let needs_attention = state == PrinterState::Stopped || worst == Some(Severity::Error);
    let will_print_now = can_submit && !needs_attention;

    Some(PrinterStatus {
        state,
        state_message: attrs.text("printer-state-message"),
        is_accepting_jobs,
        queued_job_count: attrs.integer("queued-job-count"),
        seconds_in_state: up_time.zip(attrs.integer("printer-state-change-time")).map(|(up, changed)| up - changed),
        up_time,
        config_change_time: attrs.integer("printer-config-change-time"),
        supplies: parse_supplies(response),
        state_reasons,
        can_submit,
        will_print_now,
        needs_attention,
    })
}

pub fn parse_supplies(response: &IppRequestResponse) -> Vec<Supply> {
    let Some(attrs) = Attrs::of(response, DelimiterTag::PrinterAttributes) else {
        return Vec::new();
    };
    Supply::decode(&attrs.octet_strings("printer-supply"), &attrs.texts("printer-supply-description"))
}

/// Reads every job group in a Get-Jobs response.
pub fn parse_jobs(response: &IppRequestResponse) -> Vec<Job> {
    Attrs::all_of(response, DelimiterTag::JobAttributes).iter().filter_map(parse_job).collect()
}

fn parse_job(attrs: &Attrs) -> Option<Job> {
    Some(Job {
        id: attrs.integer("job-id")?,
        uuid: attrs.text("job-uuid").map(|uuid| uuid.trim_start_matches("urn:uuid:").to_owned()),
        name: attrs.text("job-name"),
        state: attrs.enumeration("job-state").map(JobState::from_code).unwrap_or(JobState::Unknown(0)),
        state_reasons: attrs.texts("job-state-reasons").into_iter().filter(|r| r != "none").collect(),
        state_message: attrs.text("job-state-message"),
        originating_user: attrs.text("job-originating-user-name"),
        impressions: attrs.integer("job-impressions"),
        impressions_completed: attrs.integer("job-impressions-completed"),
        media_sheets_completed: attrs.integer("job-media-sheets-completed"),
        time_at_creation: attrs.integer("time-at-creation"),
        time_at_completed: attrs.integer("time-at-completed"),
    })
}

/// Reads a job id out of a Print-Job or Create-Job response.
pub fn parse_submitted_job(response: &IppRequestResponse) -> Option<Job> {
    let attrs = Attrs::of(response, DelimiterTag::JobAttributes)?;
    parse_job(&attrs)
}
