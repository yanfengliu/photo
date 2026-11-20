//! Presets: named develop settings that can be applied to any photo.

use photo_core::develop::{DevelopSettings, GroupSet};
use photo_core::meta::PresetId;
use rusqlite::{Connection, params};

use crate::catalog::Catalog;
use crate::error::{CatalogError, Entity, Result};
use crate::stored::{SETTINGS_WRITTEN, decode_settings, encode_settings};
use crate::text::natural_key;
use crate::tree::clean_name;

/// A saved develop preset.
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    /// The preset's id.
    pub id: PresetId,
    /// The group (folder) the preset is listed under.
    pub group: String,
    /// Its name, unique within the group ignoring ASCII case.
    pub name: String,
    /// The settings it was saved with, exactly.
    pub settings: DevelopSettings,
    /// The settings groups the preset applies; the rest of `settings` is to
    /// be ignored when it is applied.
    pub groups: GroupSet,
}

impl Catalog {
    /// Save a preset. Group and name are trimmed, and the name must be
    /// unique within the group, ignoring ASCII case.
    pub fn create_preset(
        &mut self,
        group: &str,
        name: &str,
        settings: &DevelopSettings,
        groups: GroupSet,
    ) -> Result<PresetId> {
        let (group, name) = (clean_name(group)?, clean_name(name)?);
        let settings = encode_settings(settings)?;
        let groups = serde_json::to_string(&groups).map_err(|source| CatalogError::Unstorable {
            what: "preset groups",
            source,
        })?;
        self.write(|tx, now| {
            require_free_name(tx, group, name, None)?;
            tx.execute(
                "INSERT INTO presets (group_name, name, settings, settings_format, groups, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![group, name, settings, SETTINGS_WRITTEN, groups, now],
            )?;
            Ok(PresetId(tx.last_insert_rowid()))
        })
    }

    /// Give a preset a new group and name (pass the current group to rename
    /// in place). The same uniqueness rule as for creating applies.
    pub fn rename_preset(&mut self, id: PresetId, group: &str, name: &str) -> Result<()> {
        let (group, name) = (clean_name(group)?, clean_name(name)?);
        self.write(|tx, _| {
            if !tx
                .prepare_cached("SELECT 1 FROM presets WHERE id = ?1")?
                .exists([id.0])?
            {
                return Err(CatalogError::not_found(Entity::Preset, id.0));
            }
            require_free_name(tx, group, name, Some(id.0))?;
            tx.execute(
                "UPDATE presets SET group_name = ?2, name = ?3 WHERE id = ?1",
                params![id.0, group, name],
            )?;
            Ok(())
        })
    }

    /// Delete a preset. Photos it was applied to keep their settings.
    pub fn delete_preset(&mut self, id: PresetId) -> Result<()> {
        self.write(|tx, _| {
            if tx.execute("DELETE FROM presets WHERE id = ?1", [id.0])? == 0 {
                return Err(CatalogError::not_found(Entity::Preset, id.0));
            }
            Ok(())
        })
    }

    /// Every preset, in natural order by group and then name.
    ///
    /// A preset stored in a form this build cannot read (written by a newer
    /// version, or damaged) is not listed, so that it cannot take the other
    /// presets down with it. It is left in the file exactly as it is, and
    /// its name in its group stays taken.
    pub fn presets(&self) -> Result<Vec<Preset>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT id, group_name, name, settings, settings_format, groups FROM presets",
        )?;
        let mut rows = stmt.query([])?;
        let mut presets = Vec::new();
        while let Some(row) = rows.next()? {
            let id = PresetId(row.get(0)?);
            let settings = decode_settings(&row.get::<_, String>(3)?, row.get(4)?);
            let groups = serde_json::from_str::<GroupSet>(&row.get::<_, String>(5)?);
            match (settings, groups) {
                (Ok(settings), Ok(groups)) => presets.push(Preset {
                    id,
                    group: row.get(1)?,
                    name: row.get(2)?,
                    settings,
                    groups,
                }),
                (Err(error), _) => log::warn!("preset {id} is not listed: {error}"),
                (_, Err(error)) => log::warn!("preset {id} is not listed: its groups: {error}"),
            }
        }
        presets.sort_by_cached_key(|preset| {
            (
                natural_key(&preset.group),
                natural_key(&preset.name),
                preset.id,
            )
        });
        Ok(presets)
    }
}

fn require_free_name(
    conn: &Connection,
    group: &str,
    name: &str,
    except: Option<i64>,
) -> Result<()> {
    let taken = conn
        .prepare_cached(
            "SELECT 1 FROM presets
             WHERE group_name = ?1 COLLATE NOCASE AND name = ?2 COLLATE NOCASE AND id IS NOT ?3",
        )?
        .exists(params![group, name, except])?;
    if taken {
        Err(CatalogError::NameTaken(name.to_string()))
    } else {
        Ok(())
    }
}
