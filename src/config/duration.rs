use std::{fmt::Formatter, time::Duration};

use serde::{de::{self, Visitor}, Deserializer};

/// Deserializes a humantime string (e.g. `30s`, `5m`) into a [`Duration`].
pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error> where D: Deserializer<'de> {
    struct DurationVisitor;

    impl<'de> Visitor<'de> for DurationVisitor {
        type Value = Duration;

        fn expecting(&self, formatter: &mut Formatter) -> Result<(), std::fmt::Error> {
            formatter.write_str("a duration string such as '30s' or '5m'")
        }

        fn visit_str<E>(self, value: &str) -> Result<Duration, E> where E: de::Error {
            humantime::parse_duration(value).map_err(|e| E::custom(format!("Invalid duration '{value}': {e}")))
        }

        fn visit_u64<E>(self, value: u64) -> Result<Duration, E> where E: de::Error {
            Ok(Duration::from_secs(value))
        }
    }

    deserializer.deserialize_any(DurationVisitor)
}
