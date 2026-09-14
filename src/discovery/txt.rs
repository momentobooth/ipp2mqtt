use std::collections::BTreeMap;

use mdns_sd::TxtProperties;

/// TXT record of a printer service.
///
/// DNS-SD keys are case insensitive, values are not, so keys are lowercased on
/// the way in and every lookup here uses the lowercased form.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TxtRecord {
    entries: BTreeMap<String, String>,
}

impl TxtRecord {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.get(key).map(String::as_str).filter(|v| !v.is_empty())
    }

    /// Reads a `T`/`F`/`U` tri-state key. `U` and anything unrecognised is unknown.
    pub fn flag(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            v if v.eq_ignore_ascii_case("t") => Some(true),
            v if v.eq_ignore_ascii_case("f") => Some(false),
            _ => None,
        }
    }

    /// Reads a comma separated key, dropping empty items left by trailing commas.
    pub fn list(&self, key: &str) -> Vec<String> {
        self.get(key)
            .map(|v| v.split(',').map(str::trim).filter(|i| !i.is_empty()).map(str::to_owned).collect())
            .unwrap_or_default()
    }

    /// Fills in keys this record is missing. Existing non-empty values win, because
    /// the `_ipp` and `_ipps` records of one printer are often not identical and the
    /// richer of the two should survive the merge.
    pub fn merge_from(&mut self, other: &TxtRecord) {
        for (key, value) in &other.entries {
            if value.is_empty() {
                continue;
            }
            match self.entries.get(key) {
                Some(existing) if !existing.is_empty() => {}
                _ => { self.entries.insert(key.clone(), value.clone()); }
            }
        }
    }

    // /////////////// //
    // Typed accessors //
    // /////////////// //

    /// Resource path of the printer URI, without leading slash (PWG 5100.14 table 3).
    /// Required by spec, but firmware omits it often enough to warrant a fallback.
    pub fn resource_path(&self) -> &str {
        self.get("rp").map(|rp| rp.trim_start_matches('/')).filter(|rp| !rp.is_empty()).unwrap_or("ipp/print")
    }

    /// Document formats the printer accepts. A truncated subset of
    /// `document-format-supported`, so it under-reports but never over-reports.
    pub fn document_formats(&self) -> Vec<String> {
        self.list("pdl")
    }

    /// Apple Raster capability tokens. Non-empty implies AirPrint-class raster.
    pub fn urf(&self) -> Vec<String> {
        self.list("urf")
    }

    /// `printer-uuid` without the `urn:uuid:` prefix. Identifies a service.
    pub fn printer_uuid(&self) -> Option<&str> {
        self.get("uuid")
    }

    /// `device-uuid` without the `urn:uuid:` prefix. Identifies the physical device,
    /// so it merges the several logical queues of a multi-function printer.
    pub fn device_uuid(&self) -> Option<&str> {
        self.get("duuid")
    }

    pub fn make_and_model(&self) -> Option<&str> {
        self.get("ty")
    }

    pub fn location(&self) -> Option<&str> {
        self.get("note")
    }

    pub fn admin_url(&self) -> Option<&str> {
        self.get("adminurl")
    }

    pub fn tls_version(&self) -> Option<&str> {
        self.get("tls")
    }

    /// Authentication the printer expects: `none`, `certificate`, `username,password`,
    /// `negotiate` or `oauth`.
    pub fn authentication(&self) -> Option<&str> {
        self.get("air")
    }

    pub fn media_kinds(&self) -> Vec<String> {
        self.list("kind")
    }

    pub fn color(&self) -> Option<bool> {
        self.flag("color")
    }

    pub fn duplex(&self) -> Option<bool> {
        self.flag("duplex")
    }

    pub fn paper_max(&self) -> Option<&str> {
        self.get("papermax")
    }
}

impl From<&TxtProperties> for TxtRecord {
    fn from(properties: &TxtProperties) -> Self {
        TxtRecord {
            entries: properties.iter().map(|p| (p.key().to_ascii_lowercase(), p.val_str().to_owned())).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pairs: &[(&str, &str)]) -> TxtRecord {
        TxtRecord { entries: pairs.iter().map(|(k, v)| (k.to_ascii_lowercase(), (*v).to_owned())).collect() }
    }

    #[test]
    fn keys_are_case_insensitive_values_are_not() {
        let txt = record(&[("PaperMax", "legal-A4")]);
        assert_eq!(txt.paper_max(), Some("legal-A4"));
    }

    #[test]
    fn resource_path_defaults_and_strips_leading_slash() {
        assert_eq!(record(&[]).resource_path(), "ipp/print");
        assert_eq!(record(&[("rp", "")]).resource_path(), "ipp/print");
        assert_eq!(record(&[("rp", "/ipp/print")]).resource_path(), "ipp/print");
        assert_eq!(record(&[("rp", "printers/Brother_HL-2030")]).resource_path(), "printers/Brother_HL-2030");
    }

    #[test]
    fn lists_ignore_trailing_commas() {
        let txt = record(&[("pdl", "image/pwg-raster,image/urf,image/jpeg,")]);
        assert_eq!(txt.document_formats(), ["image/pwg-raster", "image/urf", "image/jpeg"]);
    }

    #[test]
    fn tri_state_flags_treat_unknown_as_none() {
        assert_eq!(record(&[("Color", "T")]).color(), Some(true));
        assert_eq!(record(&[("Color", "F")]).color(), Some(false));
        assert_eq!(record(&[("Color", "U")]).color(), None);
        assert_eq!(record(&[]).color(), None);
    }

    #[test]
    fn merge_keeps_the_richer_record() {
        let mut ipps = record(&[("TLS", "1.3"), ("pdl", "")]);
        ipps.merge_from(&record(&[("pdl", "image/jpeg"), ("TLS", "1.2")]));
        assert_eq!(ipps.tls_version(), Some("1.3"));
        assert_eq!(ipps.document_formats(), ["image/jpeg"]);
    }
}
