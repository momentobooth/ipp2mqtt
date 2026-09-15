use config::{Config, Environment, File, FileFormat};
use snafu::{ResultExt, Snafu};

use super::models::Settings;

/// Config file read from the working directory, without extension so any
/// supported format is picked up.
const CONFIG_FILE: &str = "ipp2mqtt";

pub fn load_config() -> Result<Settings, ConfigError> {
    Config::builder()
        .add_source(File::with_name(CONFIG_FILE).format(FileFormat::Toml).required(false))
        .add_source(Environment::default()
            .prefix("I2M")
            .separator("_")
            .prefix_separator("_")
            .try_parsing(true)
            .with_list_parse_key("discovery.interfaces")
            .with_list_parse_key("discovery.ignoreinterfaces")
            .list_separator(","))
        .set_default("discovery.enabled", true).context(BuildSnafu)?
        .set_default("discovery.ippeverywhereonly", false).context(BuildSnafu)?
        .set_default("discovery.staleafter", "2m").context(BuildSnafu)?
        .set_default("discovery.offlineafter", "5m").context(BuildSnafu)?
        .set_default("ipp.ignoretlserrors", true).context(BuildSnafu)?
        .set_default("ipp.username", "ipp2mqtt").context(BuildSnafu)?
        .set_default("ipp.connecttimeout", "3s").context(BuildSnafu)?
        .set_default("ipp.polltimeout", "8s").context(BuildSnafu)?
        .set_default("ipp.jobtimeout", "5m").context(BuildSnafu)?
        .set_default("ipp.pollinterval", "5s").context(BuildSnafu)?
        .set_default("ipp.supplyinterval", "60s").context(BuildSnafu)?
        .set_default("mqtt.host", "localhost").context(BuildSnafu)?
        .set_default("mqtt.port", 1883).context(BuildSnafu)?
        .set_default("mqtt.secure", false).context(BuildSnafu)?
        .set_default("mqtt.ignoretlserrors", false).context(BuildSnafu)?
        .set_default("mqtt.username", "").context(BuildSnafu)?
        .set_default("mqtt.password", "").context(BuildSnafu)?
        .set_default("mqtt.clientid", "ipp2mqtt").context(BuildSnafu)?
        .set_default("mqtt.roottopic", "ipp2mqtt").context(BuildSnafu)?
        .set_default("mqtt.qos", 1).context(BuildSnafu)?
        .set_default("mqtt.keepalive", "30s").context(BuildSnafu)?
        .set_default("mqtt.cleansession", true).context(BuildSnafu)?
        .set_default("mqtt.maxpacketsize", 262144).context(BuildSnafu)?
        .build().context(BuildSnafu)?
        .try_deserialize().context(DeserializeSnafu)
}

// ////// //
// Errors //
// ////// //

#[derive(Debug, Snafu)]
pub enum ConfigError {
    #[snafu(display("Could not build configuration: {source}"))]
    Build { source: config::ConfigError },

    #[snafu(display("Could not read configuration: {source}"))]
    Deserialize { source: config::ConfigError },
}
