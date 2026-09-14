use std::collections::BTreeMap;

use serde::Serialize;

/// One marker supply, decoded from a `printer-supply` octet string.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Supply {
    pub description: Option<String>,
    /// `prtMarkerSuppliesType`, for example `ink`, `toner`, `wasteInk`, `ribbon`.
    pub supply_type: Option<String>,
    pub colorant: Option<String>,
    pub unit: Option<String>,
    pub level: Option<i32>,
    pub max_capacity: Option<i32>,
    /// Remaining percentage, already inverted for receptacles. `None` when the
    /// printer reports a sentinel rather than a measurement.
    pub percent: Option<u8>,
    /// A receptacle is filled rather than consumed, so its level counts upward.
    pub is_receptacle: bool,
    /// The printer knows some supply remains but not how much (`level = -3`).
    pub some_remaining: bool,
}

impl Supply {
    /// Decodes `printer-supply` paired with `printer-supply-description`. The two
    /// attributes correspond by position.
    pub fn decode(values: &[String], descriptions: &[String]) -> Vec<Supply> {
        values.iter().enumerate().map(|(index, value)| {
            let fields = decode_fields(value);
            let description = descriptions.get(index).map(String::as_str).filter(|d| !d.is_empty()).map(str::to_owned);
            Supply::from_fields(&fields, description)
        }).collect()
    }

    fn from_fields(fields: &BTreeMap<String, String>, description: Option<String>) -> Supply {
        let level = fields.get("level").and_then(|v| v.parse::<i32>().ok());
        let max_capacity = fields.get("maxcapacity").and_then(|v| v.parse::<i32>().ok());
        let unit = fields.get("unit").cloned();
        let is_receptacle = fields.get("class").is_some_and(|c| c.eq_ignore_ascii_case("receptacleThatIsFilled"));

        Supply {
            supply_type: fields.get("type").cloned(),
            colorant: fields.get("colorantname").cloned().filter(|c| !is_colourless(c)),
            percent: percentage(level, max_capacity, unit.as_deref(), is_receptacle),
            some_remaining: level == Some(-3),
            unit,
            level: level.filter(|l| *l >= 0),
            max_capacity: max_capacity.filter(|c| *c >= 0),
            is_receptacle,
            description,
        }
    }
}

/// Splits one supply string into its key/value pairs.
///
/// Order is not fixed and differs between implementations, so this always builds
/// a map rather than reading positionally. `:` is accepted alongside `=` because
/// the published PWG 5100.13 figures use it in places.
fn decode_fields(value: &str) -> BTreeMap<String, String> {
    value.split(';')
        .filter(|pair| !pair.trim().is_empty())
        .filter_map(|pair| pair.split_once('=').or_else(|| pair.split_once(':')))
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect()
}

/// A colorant name that means "this supply has no colour".
fn is_colourless(colorant: &str) -> bool {
    ["no-color", "none", "unknown"].iter().any(|v| colorant.eq_ignore_ascii_case(v))
}

/// Remaining percentage.
///
/// Negative levels are sentinels (`-1` other, `-2` unknown, `-3` some remaining)
/// and never become a number. A receptacle reports how full it is, so its
/// remaining capacity is the complement.
fn percentage(level: Option<i32>, max_capacity: Option<i32>, unit: Option<&str>, is_receptacle: bool) -> Option<u8> {
    let level = level.filter(|l| *l >= 0)?;

    let percent = match max_capacity {
        Some(capacity) if capacity > 0 => (100 * i64::from(level) / i64::from(capacity)) as i32,
        // Without a usable capacity the unit can still define the scale.
        _ if unit.is_some_and(|u| u.eq_ignore_ascii_case("percent")) && level <= 100 => level,
        _ => return None,
    };

    let percent = percent.clamp(0, 100);
    Some(if is_receptacle { 100 - percent } else { percent } as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(value: &str) -> Supply {
        Supply::decode(&[value.to_owned()], &[]).remove(0)
    }

    #[test]
    fn decodes_the_cups_key_order() {
        let supply = one("index=2;class=supplyThatIsConsumed;type=ink;unit=percent;maxcapacity=100;level=75;colorantname=black;");
        assert_eq!(supply.supply_type.as_deref(), Some("ink"));
        assert_eq!(supply.colorant.as_deref(), Some("black"));
        assert_eq!(supply.percent, Some(75));
        assert!(!supply.is_receptacle);
    }

    #[test]
    fn decodes_the_hp_key_order_without_an_index() {
        let supply = one("type=inkCartridge;maxcapacity=100;level=20;class=supplyThatIsConsumed;unit=percent;colorantname=magenta;");
        assert_eq!(supply.supply_type.as_deref(), Some("inkCartridge"));
        assert_eq!(supply.colorant.as_deref(), Some("magenta"));
        assert_eq!(supply.percent, Some(20));
    }

    #[test]
    fn a_receptacle_reports_remaining_capacity() {
        let supply = one("index=1;class=receptacleThatIsFilled;type=wasteInk;unit=percent;maxcapacity=100;level=25;colorantname=unknown;");
        assert!(supply.is_receptacle);
        assert_eq!(supply.percent, Some(75));
        assert_eq!(supply.colorant, None);
    }

    #[test]
    fn sentinel_levels_never_become_a_number() {
        for level in [-1, -2, -3] {
            let supply = one(&format!("type=ink;maxcapacity=100;level={level};unit=percent;"));
            assert_eq!(supply.percent, None, "level {level} should not yield a percentage");
            assert_eq!(supply.level, None);
        }
        assert!(one("type=ink;maxcapacity=100;level=-3;unit=percent;").some_remaining);
    }

    #[test]
    fn an_unknown_capacity_falls_back_to_the_percent_unit() {
        assert_eq!(one("type=ink;maxcapacity=-2;level=42;unit=percent;").percent, Some(42));
        assert_eq!(one("type=ink;maxcapacity=-2;level=42;unit=tenthsOfMilliliters;").percent, None);
    }

    #[test]
    fn tolerates_the_colon_separator_from_the_published_figures() {
        let supply = one("type=fuser; maxcapacity=100; level=89; unit:percent; colorantname=no-color; class:other;");
        assert_eq!(supply.percent, Some(89));
        assert_eq!(supply.colorant, None);
    }

    #[test]
    fn descriptions_pair_by_position_and_empty_ones_are_dropped() {
        let values = vec!["type=ink;level=10;maxcapacity=100;".to_owned(), "type=ink;level=20;maxcapacity=100;".to_owned()];
        let supplies = Supply::decode(&values, &["Black Ink".to_owned(), String::new()]);
        assert_eq!(supplies[0].description.as_deref(), Some("Black Ink"));
        assert_eq!(supplies[1].description, None);
    }

    #[test]
    fn absolute_capacities_scale_to_a_percentage() {
        assert_eq!(one("type=ribbon;maxcapacity=500;level=125;unit=sheets;").percent, Some(25));
    }
}
