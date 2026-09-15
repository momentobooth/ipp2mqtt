use std::time::Duration;

use serde::Deserialize;

use crate::config::secret::Secret;

// When changing anything here, make sure to add
// #[serde(alias = "ihavenounderscores")]
// where needed, so it can be read from the ENV vars.

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub discovery: Discovery,
    pub ipp: Ipp,
    pub mqtt: Mqtt,
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

#[derive(Debug, Deserialize)]
pub struct Ipp {
    /// Printers almost universally ship self-signed certificates, so `ipps`
    /// verification is off by default.
    #[serde(alias = "ignoretlserrors")]
    pub ignore_tls_errors: bool,

    /// Sent as `requesting-user-name` on every operation.
    #[serde(alias = "username")]
    pub user_name: String,

    #[serde(alias = "connecttimeout", deserialize_with = "super::duration::deserialize")]
    pub connect_timeout: Duration,

    #[serde(alias = "polltimeout", deserialize_with = "super::duration::deserialize")]
    pub poll_timeout: Duration,

    /// Longer than the poll timeout: a printer may hold the connection while it
    /// renders a document.
    #[serde(alias = "jobtimeout", deserialize_with = "super::duration::deserialize")]
    pub job_timeout: Duration,

    /// How often printer status is polled.
    #[serde(alias = "pollinterval", deserialize_with = "super::duration::deserialize")]
    pub poll_interval: Duration,

    /// How often supplies and loaded media are refreshed.
    #[serde(alias = "supplyinterval", deserialize_with = "super::duration::deserialize")]
    pub supply_interval: Duration,
}

#[derive(Debug, Deserialize)]
pub struct Mqtt {
    pub host: String,
    pub port: u16,
    pub secure: bool,

    /// Accept any broker certificate. Needed for a broker with a self-signed
    /// certificate and no CA file to point at.
    #[serde(alias = "ignoretlserrors")]
    pub ignore_tls_errors: bool,

    /// PEM file holding the CA that signed the broker certificate.
    #[serde(alias = "cacertificate")]
    pub ca_certificate: Option<String>,

    pub username: String,
    pub password: Secret,

    #[serde(alias = "clientid")]
    pub client_id: String,

    #[serde(alias = "roottopic")]
    pub root_topic: String,

    /// 0, 1 or 2.
    pub qos: u8,

    #[serde(alias = "keepalive", deserialize_with = "super::duration::deserialize")]
    pub keep_alive: Duration,

    /// A clean session makes the broker forget subscriptions on reconnect. The
    /// client re-applies them itself either way.
    #[serde(alias = "cleansession")]
    pub clean_session: bool,

    /// rumqttc defaults this to 10 KiB, which a status payload can exceed.
    #[serde(alias = "maxpacketsize")]
    pub max_packet_size: usize,
}
