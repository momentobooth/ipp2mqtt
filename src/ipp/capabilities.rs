use ipp::{model::DelimiterTag, request::IppRequestResponse};

use super::{attrs::Attrs, models::PrinterCapabilities};

/// Fetched once per printer and refreshed only when its configuration changes.
///
/// Requested by name rather than through the `all` or `printer-description`
/// group shortcuts: those pull in `media-col-database` and friends, which run to
/// tens of kilobytes and are not needed here.
pub const CAPABILITY_ATTRIBUTES: &[&str] = &[
    "printer-name",
    "printer-info",
    "printer-make-and-model",
    "printer-location",
    "printer-uuid",
    "printer-device-id",
    "printer-more-info",
    "printer-firmware-string-version",
    "ipp-versions-supported",
    "ipp-features-supported",
    "operations-supported",
    "document-format-supported",
    "document-format-default",
    "document-format-preferred",
    "urf-supported",
    "pwg-raster-document-type-supported",
    "pwg-raster-document-resolution-supported",
    "pwg-raster-document-sheet-back",
    "media-supported",
    "media-default",
    "media-ready",
    "print-color-mode-supported",
    "print-quality-supported",
    "sides-supported",
    "printer-resolution-supported",
    "copies-supported",
    "identify-actions-supported",
    "printer-config-change-time",
];

pub fn parse(response: &IppRequestResponse) -> Option<PrinterCapabilities> {
    let attrs = Attrs::of(response, DelimiterTag::PrinterAttributes)?;

    Some(PrinterCapabilities {
        name: attrs.text("printer-name"),
        info: attrs.text("printer-info"),
        make_and_model: attrs.text("printer-make-and-model"),
        location: attrs.text("printer-location"),
        uuid: attrs.text("printer-uuid").map(|uuid| uuid.trim_start_matches("urn:uuid:").to_owned()),
        device_id: attrs.text("printer-device-id"),
        more_info: attrs.text("printer-more-info"),
        firmware_version: attrs.text("printer-firmware-string-version"),
        ipp_versions: attrs.texts("ipp-versions-supported"),
        ipp_features: attrs.texts("ipp-features-supported"),
        operations: attrs.enumerations("operations-supported"),
        document_formats: attrs.texts("document-format-supported"),
        document_format_default: attrs.text("document-format-default"),
        document_format_preferred: attrs.text("document-format-preferred"),
        urf: attrs.texts("urf-supported"),
        pwg_raster_types: attrs.texts("pwg-raster-document-type-supported"),
        pwg_raster_resolutions: attrs.resolutions("pwg-raster-document-resolution-supported"),
        pwg_raster_sheet_back: attrs.text("pwg-raster-document-sheet-back"),
        media_supported: attrs.texts("media-supported"),
        media_default: attrs.text("media-default"),
        media_ready: attrs.texts("media-ready"),
        print_color_modes: attrs.texts("print-color-mode-supported"),
        print_qualities: attrs.enumerations("print-quality-supported"),
        sides: attrs.texts("sides-supported"),
        resolutions: attrs.resolutions("printer-resolution-supported"),
        copies_max: attrs.range_max("copies-supported"),
        identify_actions: attrs.texts("identify-actions-supported"),
        config_change_time: attrs.integer("printer-config-change-time"),
    })
}
