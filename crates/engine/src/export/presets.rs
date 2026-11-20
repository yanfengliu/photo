//! Named export presets, kept in the catalog as one JSON setting: a map
//! from name to [`ExportSettings`]. Every field of the settings reads its
//! default when missing, so presets saved by an older build still load;
//! stored text that cannot be read at all is never overwritten.

use std::collections::BTreeMap;

use photo_export::ExportSettings;

use crate::engine::Engine;
use crate::error::EngineError;

/// The catalog setting the presets are stored under.
const KEY: &str = "export.presets";

impl Engine {
    /// The saved export presets, in name order. Reads the catalog.
    pub fn export_presets(&self) -> Result<Vec<(String, ExportSettings)>, EngineError> {
        Ok(self.stored_export_presets()?.into_iter().collect())
    }

    /// Save `settings` as the export preset `name` (trimmed), replacing a
    /// preset of that name.
    pub fn save_export_preset(
        &mut self,
        name: &str,
        settings: &ExportSettings,
    ) -> Result<(), EngineError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(EngineError::EmptyName);
        }
        let mut presets = self.stored_export_presets()?;
        presets.insert(name.to_string(), settings.clone());
        self.store_export_presets(&presets)?;
        self.toasts.info(format!("Save Export Preset \"{name}\""));
        Ok(())
    }

    /// Delete the export preset `name`. Returns false when there is none.
    pub fn delete_export_preset(&mut self, name: &str) -> Result<bool, EngineError> {
        let mut presets = self.stored_export_presets()?;
        if presets.remove(name.trim()).is_none() {
            return Ok(false);
        }
        self.store_export_presets(&presets)?;
        self.toasts.info("Delete Export Preset");
        Ok(true)
    }

    fn stored_export_presets(&self) -> Result<BTreeMap<String, ExportSettings>, EngineError> {
        match self.catalog.setting(KEY)? {
            None => Ok(BTreeMap::new()),
            Some(json) => decode(&json),
        }
    }

    /// Store the presets, refusing any that would not read back (a number
    /// that is not finite is written as `null`).
    fn store_export_presets(
        &mut self,
        presets: &BTreeMap<String, ExportSettings>,
    ) -> Result<(), EngineError> {
        let json = serde_json::to_string(presets)
            .map_err(|error| EngineError::ExportPresets(error.to_string()))?;
        decode(&json)?;
        Ok(self.catalog.set_setting(KEY, &json)?)
    }
}

fn decode(json: &str) -> Result<BTreeMap<String, ExportSettings>, EngineError> {
    serde_json::from_str(json).map_err(|error| EngineError::ExportPresets(error.to_string()))
}
