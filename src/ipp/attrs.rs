use ipp::{attribute::IppAttributeGroup, model::DelimiterTag, request::IppRequestResponse, value::IppValue};

/// Tolerant reader over one attribute group.
///
/// Nothing here returns `Result`: a printer that lies about one attribute must
/// not cost us the other forty. Callers decide what a missing value means.
#[derive(Debug, Clone, Copy)]
pub struct Attrs<'a>(&'a IppAttributeGroup);

impl<'a> Attrs<'a> {
    /// Reads the first group with the given tag, falling back to the operation
    /// attributes because firmware does misfile values there.
    pub fn of(response: &'a IppRequestResponse, tag: DelimiterTag) -> Option<Self> {
        response.attributes().first_of(tag)
            .or_else(|| response.attributes().first_of(DelimiterTag::OperationAttributes))
            .map(Attrs)
    }

    /// Every group with the given tag. Get-Jobs answers with one group per job.
    pub fn all_of(response: &'a IppRequestResponse, tag: DelimiterTag) -> Vec<Self> {
        response.attributes().groups_of(tag).map(Attrs).collect()
    }

    pub fn raw(&self, name: &str) -> Option<&'a IppValue> {
        self.0.get(name).map(|attribute| attribute.value())
    }

    /// Normalises the 1setOf/single split. The parser collapses a one element set
    /// to a bare value, so the same attribute on the same printer arrives both
    /// ways depending on how many values it happens to carry.
    fn values(&self, name: &str) -> impl Iterator<Item = &'a IppValue> {
        self.raw(name).filter(|value| !is_out_of_band(value)).into_iter().flatten()
    }

    fn first(&self, name: &str) -> Option<&'a IppValue> {
        self.values(name).next()
    }

    // //////////////// //
    // Typed accessors  //
    // //////////////// //

    /// Any text-shaped value, with the language tag dropped. Printers are casual
    /// about which of the text/name and with/without-language pairs they use.
    pub fn text(&self, name: &str) -> Option<String> {
        self.first(name).and_then(text_of)
    }

    pub fn texts(&self, name: &str) -> Vec<String> {
        self.values(name).filter_map(text_of).collect()
    }

    pub fn integer(&self, name: &str) -> Option<i32> {
        self.first(name).and_then(integer_of)
    }

    pub fn integers(&self, name: &str) -> Vec<i32> {
        self.values(name).filter_map(integer_of).collect()
    }

    /// Accepts `Integer` as well as `Enum`; some firmware tags enums as integers.
    pub fn enumeration(&self, name: &str) -> Option<i32> {
        self.integer(name)
    }

    pub fn enumerations(&self, name: &str) -> Vec<i32> {
        self.integers(name)
    }

    /// Tri-state: an unrecognised shape is unknown, never `false`.
    pub fn boolean(&self, name: &str) -> Option<bool> {
        match self.first(name)? {
            IppValue::Boolean(value) => Some(*value),
            IppValue::Integer(0) => Some(false),
            IppValue::Integer(1) => Some(true),
            value => match text_of(value)?.as_str() {
                v if v.eq_ignore_ascii_case("true") || v.eq_ignore_ascii_case("t") => Some(true),
                v if v.eq_ignore_ascii_case("false") || v.eq_ignore_ascii_case("f") => Some(false),
                _ => None,
            },
        }
    }

    /// Octet strings decoded lossily. Never fail a whole poll on one bad byte.
    pub fn octet_strings(&self, name: &str) -> Vec<String> {
        self.values(name)
            .filter_map(|value| match value {
                IppValue::OctetString(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
                other => text_of(other),
            })
            .collect()
    }

    /// Resolutions rendered as `600dpi` or `300x600dpi`.
    pub fn resolutions(&self, name: &str) -> Vec<String> {
        self.values(name)
            .filter_map(|value| match value {
                IppValue::Resolution { cross_feed, feed, units } => {
                    // RFC 8011: 3 is dots per inch, 4 is dots per centimetre.
                    let unit = if *units == 4 { "dpcm" } else { "dpi" };
                    Some(if cross_feed == feed { format!("{feed}{unit}") } else { format!("{cross_feed}x{feed}{unit}") })
                }
                _ => None,
            })
            .collect()
    }

    /// Upper bound of a `rangeOfInteger`, or a plain integer used in its place.
    pub fn range_max(&self, name: &str) -> Option<i32> {
        match self.first(name)? {
            IppValue::RangeOfInteger { max, .. } => Some(*max),
            value => integer_of(value),
        }
    }
}

/// Out-of-band values stand in place of a real one and must never be read as
/// data. The crate only maps `no-value` onto a variant of its own; `unsupported`
/// and `unknown` fall through to `IppValue::Other`.
fn is_out_of_band(value: &IppValue) -> bool {
    match value {
        IppValue::NoValue => true,
        IppValue::Other { tag, .. } => (0x10..=0x1f).contains(tag),
        _ => false,
    }
}

fn text_of(value: &IppValue) -> Option<String> {
    match value {
        IppValue::TextWithoutLanguage(text) => Some(text.to_string()),
        IppValue::NameWithoutLanguage(name) => Some(name.to_string()),
        IppValue::TextWithLanguage { text, .. } => Some(text.to_string()),
        IppValue::NameWithLanguage { name, .. } => Some(name.to_string()),
        IppValue::Keyword(value) => Some(value.to_string()),
        IppValue::Uri(value) => Some(value.to_string()),
        IppValue::MimeMediaType(value) => Some(value.to_string()),
        IppValue::Charset(value) => Some(value.to_string()),
        IppValue::NaturalLanguage(value) => Some(value.to_string()),
        IppValue::UriScheme(value) => Some(value.to_string()),
        _ => None,
    }
}

fn integer_of(value: &IppValue) -> Option<i32> {
    match value {
        IppValue::Integer(value) | IppValue::Enum(value) => Some(*value),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use ipp::attribute::IppAttribute;

    use super::*;

    fn group(attributes: Vec<(&str, IppValue)>) -> IppAttributeGroup {
        let mut group = IppAttributeGroup::new(DelimiterTag::PrinterAttributes);
        for (name, value) in attributes {
            group.attributes_mut().push(IppAttribute::with_name(name, value).unwrap());
        }
        group
    }

    fn keyword(value: &str) -> IppValue {
        IppValue::Keyword(value.try_into().unwrap())
    }

    #[test]
    fn a_single_value_and_a_set_read_the_same_way() {
        let single = group(vec![("printer-state-reasons", keyword("none"))]);
        let many = group(vec![("printer-state-reasons", IppValue::Array(vec![keyword("toner-low-report"), keyword("media-empty-error")]))]);

        assert_eq!(Attrs(&single).texts("printer-state-reasons"), ["none"]);
        assert_eq!(Attrs(&many).texts("printer-state-reasons"), ["toner-low-report", "media-empty-error"]);
        // A singular read of a set takes the first value rather than failing.
        assert_eq!(Attrs(&many).text("printer-state-reasons").as_deref(), Some("toner-low-report"));
    }

    #[test]
    fn out_of_band_values_read_as_absent() {
        for tag in [0x10, 0x12] {
            let g = group(vec![("printer-current-time", IppValue::Other { tag, data: Default::default() })]);
            assert_eq!(Attrs(&g).text("printer-current-time"), None, "tag {tag:#04x} should read as absent");
        }
        let g = group(vec![("printer-current-time", IppValue::NoValue)]);
        assert_eq!(Attrs(&g).text("printer-current-time"), None);
    }

    #[test]
    fn enums_are_accepted_where_integers_were_sent_and_the_reverse() {
        let as_enum = group(vec![("printer-state", IppValue::Enum(4))]);
        let as_integer = group(vec![("printer-state", IppValue::Integer(4))]);
        assert_eq!(Attrs(&as_enum).enumeration("printer-state"), Some(4));
        assert_eq!(Attrs(&as_integer).enumeration("printer-state"), Some(4));
    }

    #[test]
    fn booleans_are_tri_state_and_read_several_shapes() {
        assert_eq!(Attrs(&group(vec![("x", IppValue::Boolean(true))])).boolean("x"), Some(true));
        assert_eq!(Attrs(&group(vec![("x", IppValue::Integer(0))])).boolean("x"), Some(false));
        assert_eq!(Attrs(&group(vec![("x", keyword("true"))])).boolean("x"), Some(true));
        // Unrecognised is unknown, never false.
        assert_eq!(Attrs(&group(vec![("x", keyword("maybe"))])).boolean("x"), None);
        assert_eq!(Attrs(&group(vec![])).boolean("x"), None);
    }

    #[test]
    fn resolutions_use_the_unit_the_printer_reported() {
        let g = group(vec![("printer-resolution-supported", IppValue::Array(vec![
            IppValue::Resolution { cross_feed: 600, feed: 600, units: 3 },
            IppValue::Resolution { cross_feed: 300, feed: 600, units: 3 },
            IppValue::Resolution { cross_feed: 120, feed: 120, units: 4 },
        ]))]);
        assert_eq!(Attrs(&g).resolutions("printer-resolution-supported"), ["600dpi", "300x600dpi", "120dpcm"]);
    }

    #[test]
    fn a_range_yields_its_maximum_and_a_plain_integer_stands_in() {
        assert_eq!(Attrs(&group(vec![("copies-supported", IppValue::RangeOfInteger { min: 1, max: 999 })])).range_max("copies-supported"), Some(999));
        assert_eq!(Attrs(&group(vec![("copies-supported", IppValue::Integer(9))])).range_max("copies-supported"), Some(9));
    }

    #[test]
    fn octet_strings_survive_invalid_utf8() {
        let g = group(vec![("printer-supply", IppValue::OctetString(vec![b'a', 0xff, b'b'].into()))]);
        assert_eq!(Attrs(&g).octet_strings("printer-supply").len(), 1);
    }
}
