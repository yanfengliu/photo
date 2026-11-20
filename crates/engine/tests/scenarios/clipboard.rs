//! Copy and paste, sync, the previous photo's settings, and reset.

use photo_core::develop::{Adj, DevelopSettings, GroupSet, SettingsGroup};
use photo_core::meta::{FileKind, PhotoId};
use photo_engine::{EngineError, Target};
use photo_testkit::TempLibrary;

use crate::common::Studio;

fn studio() -> (TempLibrary, Studio) {
    let library = TempLibrary::new();
    let mut studio = Studio::new();
    studio.import_library(&library);
    (library, studio)
}

/// Give `photo` exposure, vibrance and a crop, as one discrete edit.
fn edit(studio: &mut Studio, photo: PhotoId) -> DevelopSettings {
    let mut settings = studio.engine.develop_settings(photo).unwrap();
    settings.tone.exposure = 0.7;
    settings.presence.vibrance = 35.0;
    settings.crop.rect = Some(photo_core::develop::CropRect {
        cx: 0.5,
        cy: 0.5,
        width: 0.5,
        height: 0.5,
    });
    studio.engine.open_develop(photo).unwrap();
    studio.engine.apply_settings(settings).unwrap();
    studio.engine.close_develop();
    studio.engine.develop_settings(photo).unwrap()
}

#[test]
fn pasting_applies_only_the_copied_groups_to_every_target_in_one_step() {
    let (_library, mut studio) = studio();
    let (dawn, lake, meadow) = (
        studio.id("dawn.jpg"),
        studio.id("lake.png"),
        studio.id("meadow.tif"),
    );
    let edited = edit(&mut studio, dawn);
    assert!(matches!(
        studio.engine.paste_settings(Target::Active),
        Err(EngineError::NothingToPaste)
    ));
    studio.engine.select_only(dawn);
    studio
        .engine
        .copy_settings(GroupSet::default_copy())
        .unwrap();
    let clipboard = studio.engine.clipboard().unwrap();
    assert_eq!(clipboard.from_photo(), dawn);
    assert_eq!(clipboard.settings(), &edited);

    studio.engine.select_only(lake);
    studio.engine.toggle_selected(meadow);
    studio.engine.paste_settings(Target::Selected).unwrap();
    assert_eq!(studio.engine.undo_label(), Some("Paste Settings"));
    for photo in [lake, meadow] {
        let settings = studio.engine.develop_settings(photo).unwrap();
        assert_eq!(settings.tone.exposure, 0.7);
        assert_eq!(settings.presence.vibrance, 35.0);
        assert_eq!(settings.crop.rect, None, "crop is not in the default copy");
    }
    studio.engine.undo().unwrap();
    for photo in [lake, meadow] {
        assert!(
            studio
                .engine
                .develop_settings(photo)
                .unwrap()
                .is_default_for(FileKind::Raster)
        );
    }
}

#[test]
fn sync_gives_the_other_selected_photos_the_active_photos_groups() {
    let (_library, mut studio) = studio();
    let (dawn, lake, ridge) = (
        studio.id("dawn.jpg"),
        studio.id("lake.png"),
        studio.id("ridge.jpg"),
    );
    edit(&mut studio, dawn);
    studio.engine.select_only(lake);
    studio.engine.toggle_selected(ridge);
    studio.engine.toggle_selected(dawn);
    assert_eq!(studio.engine.active_photo(), Some(dawn));
    let tone_only = GroupSet::NONE.with(SettingsGroup::Tone);
    studio.engine.sync_settings(tone_only).unwrap();
    for photo in [lake, ridge] {
        let settings = studio.engine.develop_settings(photo).unwrap();
        assert_eq!(settings.tone.exposure, 0.7);
        assert_eq!(settings.presence.vibrance, 0.0);
    }
    assert_eq!(studio.engine.undo_label(), Some("Sync Settings"));
    studio.engine.undo().unwrap();
    assert_eq!(
        studio.engine.develop_settings(ridge).unwrap().tone.exposure,
        0.0
    );
}

#[test]
fn paste_from_previous_takes_the_photo_active_before() {
    let (_library, mut studio) = studio();
    let (dawn, lake) = (studio.id("dawn.jpg"), studio.id("lake.png"));
    let edited = edit(&mut studio, dawn);
    assert!(matches!(
        studio.engine.paste_from_previous(Target::Active),
        Err(EngineError::NoPreviousPhoto)
    ));
    studio.engine.select_only(dawn);
    studio.engine.select_only(lake);
    assert_eq!(studio.engine.previous_photo(), Some(dawn));
    studio.engine.paste_from_previous(Target::Active).unwrap();
    assert_eq!(
        studio.engine.develop_settings(lake).unwrap(),
        edited,
        "every group, crop too"
    );
    assert_eq!(studio.engine.undo_label(), Some("Paste from Previous"));
}

#[test]
fn reset_puts_back_the_starting_settings_for_each_kind_or_one_group() {
    let (_library, mut studio) = studio();
    let (dawn, harbour) = (studio.id("dawn.jpg"), studio.id("harbour.dng"));
    edit(&mut studio, dawn);
    edit(&mut studio, harbour);
    let both = Target::Photos(vec![dawn, harbour]);

    studio
        .engine
        .reset_group(both.clone(), SettingsGroup::Presence)
        .unwrap();
    assert_eq!(studio.engine.undo_label(), Some("Reset Presence"));
    for photo in [dawn, harbour] {
        let settings = studio.engine.develop_settings(photo).unwrap();
        assert_eq!(settings.presence.vibrance, 0.0);
        assert_eq!(settings.tone.exposure, 0.7, "other groups stay");
    }

    studio.engine.reset_settings(both).unwrap();
    assert!(
        studio
            .engine
            .develop_settings(dawn)
            .unwrap()
            .is_default_for(FileKind::Raster)
    );
    let raw = studio.engine.develop_settings(harbour).unwrap();
    assert!(raw.is_default_for(FileKind::Raw));
    assert_eq!(
        raw.detail.sharpening.amount,
        DevelopSettings::RAW_SHARPEN_AMOUNT
    );
    studio.engine.undo().unwrap();
    assert_eq!(
        studio
            .engine
            .develop_settings(harbour)
            .unwrap()
            .tone
            .exposure,
        0.7
    );
    assert_eq!(
        studio
            .engine
            .develop_settings(harbour)
            .unwrap()
            .presence
            .vibrance,
        0.0
    );
}

#[test]
fn quick_develop_moves_each_selected_photo_from_where_it_is_in_one_step() {
    let (_library, mut studio) = studio();
    let (dawn, lake, harbour) = (
        studio.id("dawn.jpg"),
        studio.id("lake.png"),
        studio.id("harbour.dng"),
    );
    edit(&mut studio, dawn);
    studio.engine.select_only(dawn);
    studio.engine.toggle_selected(lake);
    studio.engine.toggle_selected(harbour);

    studio
        .engine
        .quick_adjust(Target::Selected, Adj::Exposure, 1.0 / 3.0)
        .unwrap();
    assert_eq!(
        studio.engine.undo_label(),
        Some("Quick Develop: Exposure +0.33")
    );
    let exposure =
        |studio: &Studio, photo| studio.engine.develop_settings(photo).unwrap().tone.exposure;
    assert!((exposure(&studio, dawn) - (0.7 + 1.0 / 3.0)).abs() < 1e-6);
    assert!((exposure(&studio, lake) - 1.0 / 3.0).abs() < 1e-6);
    studio
        .engine
        .quick_adjust(Target::Selected, Adj::Clarity, 20.0)
        .unwrap();
    studio.engine.undo().unwrap();
    studio.engine.undo().unwrap();
    assert_eq!(exposure(&studio, dawn), 0.7, "each press is one undo step");
    assert_eq!(exposure(&studio, lake), 0.0);

    studio
        .engine
        .quick_adjust(Target::Selected, Adj::Temperature, 10.0)
        .unwrap();
    let balance = studio.engine.develop_settings(lake).unwrap().white_balance;
    assert_eq!(balance.temp, 10.0, "a raster file's balance is relative");
    let raw = studio
        .engine
        .develop_settings(harbour)
        .unwrap()
        .white_balance;
    assert_eq!(
        raw,
        Default::default(),
        "an as-shot RAW balance is only known once decoded"
    );

    studio
        .engine
        .quick_white_balance(Target::Selected, photo_core::develop::WbPreset::Daylight)
        .unwrap();
    let raw = studio
        .engine
        .develop_settings(harbour)
        .unwrap()
        .white_balance;
    assert_eq!(raw.preset, photo_core::develop::WbPreset::Daylight);
    assert_eq!(
        studio.engine.develop_settings(lake).unwrap().white_balance,
        balance,
        "raster files keep theirs"
    );

    let before: Vec<_> = [dawn, lake, harbour]
        .iter()
        .map(|photo| studio.engine.develop_settings(*photo).unwrap())
        .collect();
    studio.engine.quick_auto_tone(Target::Selected).unwrap();
    studio.settle();
    assert_eq!(studio.engine.undo_label(), Some("Quick Develop: Auto Tone"));
    for (photo, before) in [dawn, lake, harbour].iter().zip(&before) {
        let after = studio.engine.develop_settings(*photo).unwrap();
        assert_ne!(after.tone, before.tone, "tone was set");
        assert_eq!(after.presence, before.presence, "nothing else was");
    }
    studio.engine.undo().unwrap();
    for (photo, before) in [dawn, lake, harbour].iter().zip(&before) {
        assert_eq!(&studio.engine.develop_settings(*photo).unwrap(), before);
    }

    studio
        .engine
        .quick_white_balance(Target::Selected, photo_core::develop::WbPreset::Auto)
        .unwrap();
    studio.settle();
    for photo in [dawn, lake, harbour] {
        let balance = studio.engine.develop_settings(photo).unwrap().white_balance;
        assert_eq!(balance.preset, photo_core::develop::WbPreset::Auto);
    }
    assert_eq!(
        studio.engine.undo_label(),
        Some("Quick Develop: White Balance Auto")
    );
}
