use std::time::Duration;

use serde::Deserialize;

// When changing anything here, make sure to add
// #[serde(alias = "ihavenounderscores")]
// where needed, so it can be read from the ENV vars.

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub discovery: Discovery,
    #[serde(default)]
    pub printers: Vec<StaticPrinter>,
}

#[derive(Debug, Deserialize)]
pub struct Discovery {
    pub enabled: bool,

    /// Interface names to browse on. Empty means every interface.
    #[serde(default)]
    pub interfaces: Vec<String>,

    /// Interface names to never browse on, applied after `interfaces`.
    #[serde(default, alias = "ignoreinterfaces")]
    pub ignore_interfaces: Vec<String>,

    /// Only accept printers advertising the `_print._sub` IPP Everywhere subtype.
    #[serde(alias = "ippeverywhereonly")]
    pub ipp_everywhere_only: bool,

    /// Time without an mDNS refresh after which a printer is reported as stale.
    #[serde(alias = "staleafter", deserialize_with = "super::duration::deserialize")]
    pub stale_after: Duration,

    /// Time without an mDNS refresh after which a printer is reported as offline.
    #[serde(alias = "offlineafter", deserialize_with = "super::duration::deserialize")]
    pub offline_after: Duration,
}

/// A printer that is configured by hand instead of discovered over mDNS.
#[derive(Debug, Deserialize)]
pub struct StaticPrinter {
    pub id: String,
    pub uri: String,
}
