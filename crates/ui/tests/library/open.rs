//! Opening a photo by its path in the running app, and starting the app on
//! a view: what the agent harness and `photo PATH` ask of the UI.

use egui::{Key, Modifiers, Role};
use photo_engine::Source;
use photo_testkit::TempLibrary;
use photo_ui::harness::HarnessBuilder;
use photo_ui::{Module, OpenError, PhotoApp, View};

use crate::{PATIENCE, Studio, TestResult, WINDOW, active, click_cell, files, id, settle};

#[test]
fn a_photo_is_opened_by_its_path_without_starting_the_app_again() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;

    // Something of the session that a restart would lose: an undo step and
    // a closed panel.
    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;
    harness.press_key(Key::Num3, Modifiers::NONE)?;
    harness.click(Role::DisclosureTriangle, "Folders")?;
    assert_eq!(harness.app().engine().undo_label(), Some("Set Rating to 3"));
    assert!(!harness.app().left_panels().is_open("Folders"));

    // A file the catalog does not hold is imported where it is and shown
    // in the loupe.
    let elsewhere = tempfile::tempdir()?;
    let path = photo_testkit::write_jpeg(elsewhere.path(), "extra.jpg", 96, 64);
    let opened = harness.app_mut().open_path(&path, PATIENCE)?;
    settle(&mut harness)?;
    assert!(opened.imported);
    assert_eq!(opened.file_name, "extra.jpg");
    assert_eq!(harness.app().view(), View::Loupe);
    assert_eq!(harness.app().module(), Module::Library);
    let engine = harness.app().engine();
    assert_eq!(engine.active_photo(), Some(opened.photo));
    assert_eq!(engine.selection().selected(), [opened.photo]);
    assert_eq!(engine.source(), &Source::PreviousImport);
    assert_eq!(
        engine.undo_label(),
        Some("Set Rating to 3"),
        "undo survives"
    );
    assert!(
        !harness.app().left_panels().is_open("Folders"),
        "panels survive"
    );

    // Opened again, it is found, not imported a second time.
    let again = harness.app_mut().open_path(&path, PATIENCE)?;
    assert!(!again.imported);
    assert_eq!(again.photo, opened.photo);
    assert_eq!(harness.app().engine().source_counts().all_photos, 7);

    // A photo of the catalog that the list does not show is found among
    // all photographs.
    let dawn = library
        .entries()
        .iter()
        .find(|entry| entry.path.ends_with("dawn.jpg"))
        .ok_or("the library has dawn.jpg")?
        .path
        .clone();
    let found = harness.app_mut().open_path(&dawn, PATIENCE)?;
    settle(&mut harness)?;
    assert!(!found.imported);
    assert_eq!(harness.app().engine().source(), &Source::AllPhotos);
    assert_eq!(active(&harness).as_deref(), Some("dawn.jpg"));
    assert_eq!(found.photo, id(&harness, "dawn.jpg"));

    // What cannot be opened says why, and changes nothing.
    let missing = elsewhere.path().join("nowhere.jpg");
    assert_eq!(
        harness.app_mut().open_path(&missing, PATIENCE),
        Err(OpenError::Missing(missing))
    );
    let stray = library.stray_file().to_path_buf();
    assert_eq!(
        harness.app_mut().open_path(&stray, PATIENCE),
        Err(OpenError::Unsupported(stray))
    );
    assert_eq!(active(&harness).as_deref(), Some("dawn.jpg"));
    Ok(())
}

#[test]
fn a_photo_is_opened_in_an_engine_before_there_is_an_app() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut engine = studio.options.open_engine()?;
    let lake = library
        .entries()
        .iter()
        .find(|entry| entry.path.ends_with("lake.png"))
        .ok_or("the library has lake.png")?
        .path
        .clone();
    assert_eq!(photo_ui::openable(&lake), Ok(lake.clone()));
    let stray = library.stray_file().to_path_buf();
    assert_eq!(
        photo_ui::openable(&stray),
        Err(OpenError::Unsupported(stray.clone()))
    );
    assert_eq!(
        photo_ui::open_in_engine(&mut engine, &stray, PATIENCE),
        Err(OpenError::Unsupported(stray))
    );
    assert_eq!(engine.photo_count(), 0, "a refusal imports nothing");

    let opened = photo_ui::open_in_engine(&mut engine, &lake, PATIENCE)?;
    assert!(opened.imported);
    assert_eq!(
        (opened.file_name.as_str(), &opened.path),
        ("lake.png", &lake)
    );
    assert_eq!(engine.active_photo(), Some(opened.photo));
    assert_eq!(engine.selection().selected(), [opened.photo]);

    // The app started on that engine shows it in the loupe.
    let options = studio.options.clone();
    let mut harness = HarnessBuilder::new()
        .size(WINDOW)
        .max_steps(240)
        .build(move |cc| {
            PhotoApp::new(cc, engine, options).starting_in(Module::Library, View::Loupe)
        });
    settle(&mut harness)?;
    assert_eq!(harness.app().view(), View::Loupe);
    assert_eq!(active(&harness).as_deref(), Some("lake.png"));
    Ok(())
}

#[test]
fn a_photo_opened_while_developing_is_developed() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let mut harness = studio.app_with(files(&library));
    settle(&mut harness)?;
    click_cell(&mut harness, "dawn.jpg", Modifiers::NONE)?;
    harness.press_key(Key::D, Modifiers::NONE)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().module(), Module::Develop);

    let lake = library
        .entries()
        .iter()
        .find(|entry| entry.path.ends_with("lake.png"))
        .ok_or("the library has lake.png")?
        .path
        .clone();
    let opened = harness.app_mut().open_path(&lake, PATIENCE)?;
    settle(&mut harness)?;
    assert_eq!(harness.app().module(), Module::Develop, "Develop stays");
    let session = harness.app().engine().develop().ok_or("a session")?;
    assert_eq!(session.photo(), opened.photo);
    assert_eq!(harness.app().canvas_shows().photo, Some(opened.photo));
    Ok(())
}

#[test]
fn the_app_starts_in_the_module_and_view_it_is_given() -> TestResult {
    let library = TempLibrary::new();
    let studio = Studio::new();
    let build = |module: Module, view: View| {
        let mut engine = studio.options.open_engine().expect("the catalog opens");
        if engine.photo_count() == 0 {
            crate::import(&mut engine, files(&library));
        }
        let options = studio.options.clone();
        HarnessBuilder::new()
            .size(WINDOW)
            .max_steps(240)
            .build(move |cc| PhotoApp::new(cc, engine, options).starting_in(module, view))
    };

    let mut loupe = build(Module::Library, View::Loupe);
    settle(&mut loupe)?;
    assert_eq!(
        (loupe.app().module(), loupe.app().view()),
        (Module::Library, View::Loupe)
    );
    assert!(loupe.app().engine().active_photo().is_some());
    drop(loupe);

    let mut develop = build(Module::Develop, View::Grid);
    settle(&mut develop)?;
    assert_eq!(develop.app().module(), Module::Develop);
    let active = develop.app().engine().active_photo();
    assert!(active.is_some(), "the first photo is opened");
    assert_eq!(
        develop
            .app()
            .engine()
            .develop()
            .map(|session| session.photo()),
        active
    );
    drop(develop);

    // An empty catalog has nothing to develop.
    let empty = Studio::new();
    let engine = empty.options.open_engine()?;
    let options = empty.options.clone();
    let mut harness = HarnessBuilder::new().size(WINDOW).build(move |cc| {
        PhotoApp::new(cc, engine, options).starting_in(Module::Develop, View::Grid)
    });
    harness.run()?;
    assert_eq!(harness.app().module(), Module::Library);
    Ok(())
}
