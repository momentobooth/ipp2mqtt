use std::fmt::{Debug, Formatter};

use serde::{Deserialize, Deserializer};

/// A configuration value that must not appear in logs.
///
/// The whole settings struct is logged at startup, so passwords are wrapped to
/// keep them out of that output.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Debug for Secret {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        f.write_str(if self.0.is_empty() { "\"\"" } else { "\"<redacted>\"" })
    }
}

impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error> where D: Deserializer<'de> {
        String::deserialize(deserializer).map(Secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_never_shows_its_value() {
        let secret = Secret("hunter2".to_owned());
        assert_eq!(format!("{secret:?}"), "\"<redacted>\"");
        assert!(!format!("{secret:#?}").contains("hunter2"));
        assert_eq!(secret.expose(), "hunter2");
    }

    #[test]
    fn an_empty_secret_is_distinguishable_from_a_set_one() {
        assert_eq!(format!("{:?}", Secret::default()), "\"\"");
        assert!(Secret::default().is_empty());
    }
}
