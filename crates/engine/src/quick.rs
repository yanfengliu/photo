//! Quick Develop: relative edits to a selection from the Library, each one
//! undoable step. Automatic values need each photo decoded, which happens in
//! the background; the step is recorded when every photo is done.

use std::collections::HashMap;

use photo_core::develop::{Adj, DevelopSettings, WbPreset, WhiteBalance};
use photo_core::meta::{FileKind, PhotoId};
use photo_develop::Source;

use crate::commands::Target;
use crate::engine::{Engine, JobFailure};
use crate::error::EngineError;
use crate::jobs::Lane;
use crate::stamp::Stamp;

/// What an automatic Quick Develop run computes for each photo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Auto {
    Tone,
    WhiteBalance,
}

impl Auto {
    fn label(self) -> &'static str {
        match self {
            Self::Tone => "Quick Develop: Auto Tone",
            Self::WhiteBalance => "Quick Develop: White Balance Auto",
        }
    }

    fn apply(self, source: &Source, settings: &DevelopSettings) -> DevelopSettings {
        let mut next = settings.clone();
        match self {
            Self::Tone => next.tone = photo_develop::auto_tone(source, settings),
            Self::WhiteBalance => next.white_balance = photo_develop::auto_white_balance(source),
        }
        next
    }
}

/// An automatic run waiting for its photos.
pub(crate) struct AutoRun {
    auto: Auto,
    photos: Vec<PhotoId>,
    waiting: usize,
    computed: HashMap<PhotoId, DevelopSettings>,
    failures: usize,
}

impl Engine {
    /// Move one adjustment of every target photo by `delta` from where it
    /// is (exposure by a third of a stop, contrast by a step...), held to
    /// the slider's range. Temperature and tint move a raster file's
    /// relative balance; a RAW photo with its as-shot balance is left alone
    /// for them, since its as-shot values are only known once decoded.
    pub fn quick_adjust(
        &mut self,
        target: Target,
        adj: Adj,
        delta: f32,
    ) -> Result<(), EngineError> {
        let photos = self.resolve(&target);
        let editable = self.editable_settings(&photos)?;
        let spec = adj.spec();
        let label = format!("Quick Develop: {} {}", spec.label, spec.format(delta));
        self.write_develop(&label, editable, |_, current, kind| {
            let mut next = current.clone();
            if matches!(adj, Adj::Temperature | Adj::Tint)
                && next.white_balance.preset != WbPreset::Custom
            {
                if kind == FileKind::Raw && next.white_balance.preset == WbPreset::AsShot {
                    return next;
                }
                let start = if next.white_balance.preset == WbPreset::AsShot {
                    (0.0, 0.0)
                } else {
                    (next.white_balance.temp, next.white_balance.tint)
                };
                next.white_balance = WhiteBalance {
                    preset: WbPreset::Custom,
                    temp: start.0,
                    tint: start.1,
                };
            }
            let value = adj.get(&next) + delta;
            adj.set(&mut next, adj.spec_for(kind).clamp(value));
            next
        })
    }

    /// Give every target photo a white balance preset. The illuminant
    /// presets are absolute and only apply to RAW files; raster files keep
    /// their balance for them. `Auto` decodes each photo in the background
    /// and records the step when all are done.
    pub fn quick_white_balance(
        &mut self,
        target: Target,
        preset: WbPreset,
    ) -> Result<(), EngineError> {
        if preset == WbPreset::Auto {
            return self.start_auto(&target, Auto::WhiteBalance);
        }
        let photos = self.resolve(&target);
        let editable = self.editable_settings(&photos)?;
        let label = format!("Quick Develop: White Balance {}", preset.label());
        self.write_develop(&label, editable, |_, current, kind| {
            let mut next = current.clone();
            match (preset, kind, preset.illuminant()) {
                (WbPreset::AsShot, _, _) => next.white_balance = WhiteBalance::default(),
                (_, FileKind::Raw, Some((temp, tint))) => {
                    next.white_balance = WhiteBalance { preset, temp, tint };
                }
                _ => {}
            }
            next
        })
    }

    /// Set every target photo's tone automatically. Each photo is decoded in
    /// the background; the step is recorded when all are done.
    pub fn quick_auto_tone(&mut self, target: Target) -> Result<(), EngineError> {
        self.start_auto(&target, Auto::Tone)
    }

    fn start_auto(&mut self, target: &Target, auto: Auto) -> Result<(), EngineError> {
        let photos = self.resolve(target);
        let editable = self.editable_settings(&photos)?;
        self.next_auto += 1;
        let run = self.next_auto;
        let mut work = Vec::new();
        for (photo, _, settings) in &editable.photos {
            let record = self.catalog.photo(*photo)?;
            work.push((*photo, record.path, settings.clone()));
        }
        self.autos.insert(
            run,
            AutoRun {
                auto,
                photos: work.iter().map(|(photo, _, _)| *photo).collect(),
                waiting: work.len(),
                computed: HashMap::new(),
                failures: 0,
            },
        );
        for (photo, path, settings) in work {
            let sources = self.sources.clone();
            self.submit(
                Lane::Background,
                None,
                None,
                move |_| {
                    let computed = Stamp::read(&path)
                        .map_err(|error| error.to_string())
                        .and_then(|stamp| {
                            sources
                                .decode(&path, stamp)
                                .map_err(|error| error.to_string())
                        })
                        .map(|source| auto.apply(&source, &settings));
                    Box::new(move |engine: &mut Engine| engine.auto_done(run, photo, computed.ok()))
                },
                move |engine, failure| {
                    if let JobFailure::Panicked(message) = failure {
                        log::error!("automatic settings failed: {message}");
                    }
                    engine.auto_done(run, photo, None)
                },
            );
        }
        self.auto_progress(run);
        Ok(())
    }

    fn auto_done(&mut self, run: u64, photo: PhotoId, computed: Option<DevelopSettings>) -> bool {
        if let Some(state) = self.autos.get_mut(&run) {
            state.waiting = state.waiting.saturating_sub(1);
            match computed {
                Some(settings) => {
                    state.computed.insert(photo, settings);
                }
                None => state.failures += 1,
            }
        }
        self.auto_progress(run);
        true
    }

    /// Once every photo of the run is through, lay what was computed over
    /// the photos' settings as they are now (they may have changed while
    /// the run was decoding) and record it as one step.
    fn auto_progress(&mut self, run: u64) {
        if self.autos.get(&run).is_none_or(|state| state.waiting > 0) {
            return;
        }
        let Some(state) = self.autos.remove(&run) else {
            return;
        };
        if state.failures > 0 {
            self.toasts.error(format!(
                "{} photos could not be read for automatic settings",
                state.failures
            ));
        }
        let written = self.editable_settings(&state.photos).and_then(|editable| {
            let (auto, computed) = (state.auto, &state.computed);
            self.write_develop(auto.label(), editable, |photo, current, _| {
                let mut next = current.clone();
                if let Some(found) = computed.get(&photo) {
                    match auto {
                        Auto::Tone => next.tone = found.tone,
                        Auto::WhiteBalance => next.white_balance = found.white_balance,
                    }
                }
                next
            })
        });
        if let Err(error) = written {
            self.toasts
                .error(format!("Could not apply automatic settings: {error}"));
        }
    }
}
