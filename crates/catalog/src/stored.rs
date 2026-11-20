//! Develop settings and smart rules as the catalog stores them: JSON text,
//! with the number of the format it is written in kept beside it.
//!
//! Text in a newer format than this build knows is never decoded, because
//! decoding would drop whatever this build does not know, and callers never
//! replace it. Older formats decode as they are; what they lack takes its
//! default.

use photo_core::develop::{DevelopSettings, SETTINGS_FORMAT};

use crate::error::{CatalogError, Result};
use crate::smart::SmartRules;

const SETTINGS: &str = "develop settings";
const RULES: &str = "smart collection rules";

/// The format number stored beside the settings this build writes.
pub(crate) const SETTINGS_WRITTEN: i64 = SETTINGS_FORMAT as i64;

/// The format number stored beside the rules this build writes.
pub(crate) const RULES_WRITTEN: i64 = SmartRules::FORMAT as i64;

/// Settings as the JSON the catalog stores, in [`SETTINGS_WRITTEN`] format.
/// Refuses settings that would not read back (a NaN or infinity is written
/// as `null`), so that nothing unreadable is ever stored.
pub(crate) fn encode_settings(settings: &DevelopSettings) -> Result<String> {
    let unstorable = |source| CatalogError::Unstorable {
        what: SETTINGS,
        source,
    };
    let json = serde_json::to_string(settings).map_err(unstorable)?;
    serde_json::from_str::<DevelopSettings>(&json).map_err(unstorable)?;
    Ok(json)
}

/// Stored settings back into settings. Fails with
/// [`CatalogError::NewerFormat`] for a format this build does not know, and
/// with [`CatalogError::Corrupt`] if the text does not parse; neither is
/// ever papered over with defaults, because the next write would then
/// destroy the stored edit.
pub(crate) fn decode_settings(json: &str, format: i64) -> Result<DevelopSettings> {
    require_settings_format(format)?;
    serde_json::from_str(json).map_err(|source| CatalogError::Corrupt {
        what: SETTINGS,
        source,
    })
}

/// Fail with [`CatalogError::NewerFormat`] if settings stored in `format`
/// may not be read or replaced by this build.
pub(crate) fn require_settings_format(format: i64) -> Result<()> {
    require_format(SETTINGS, format, SETTINGS_FORMAT)
}

/// Rules as the JSON the catalog stores, in [`RULES_WRITTEN`] format. Rules
/// that do not validate are refused.
pub(crate) fn encode_rules(rules: &SmartRules) -> Result<String> {
    rules.validate()?;
    serde_json::to_string(rules).map_err(|source| CatalogError::Unstorable {
        what: RULES,
        source,
    })
}

/// Stored rules back into rules; fails as [`decode_settings`] does.
pub(crate) fn decode_rules(json: &str, format: i64) -> Result<SmartRules> {
    require_rules_format(format)?;
    serde_json::from_str(json).map_err(|source| CatalogError::Corrupt {
        what: RULES,
        source,
    })
}

/// Fail with [`CatalogError::NewerFormat`] if rules stored in `format` may
/// not be read or replaced by this build.
pub(crate) fn require_rules_format(format: i64) -> Result<()> {
    require_format(RULES, format, SmartRules::FORMAT)
}

fn require_format(what: &'static str, stored: i64, supported: u32) -> Result<()> {
    if stored > i64::from(supported) {
        Err(CatalogError::NewerFormat {
            what,
            found: u32::try_from(stored).unwrap_or(u32::MAX),
            supported,
        })
    } else {
        Ok(())
    }
}
