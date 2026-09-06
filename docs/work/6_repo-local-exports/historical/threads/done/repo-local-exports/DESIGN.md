# Save-as-copy exports move into the repo (repo-local-exports)

Date: 2026-06-11. User request: "the edited files should not be saved in the original path where the photo is provided. It should be saved in this repo path, and the path should be gitignored."

## Diagnosis

`edit::edited_save_path` produced `<source dir>/<stem>_edited.<ext>` via `Path::with_file_name`, so Ctrl+S/save-as-copy wrote the export next to the original — for this user's library that is the camera's SD card (`E:\DCIM\100MSDCF\`). Writing app output onto removable source media is wrong in both directions: it pollutes the card and the export vanishes with it.

## Design

`edited_save_path` now resolves through `repo::photo_repo_root()`: with a repo root, exports land in `<repo>/edited/<stem>_edited.<ext>` (RAW sources still export as `.png`); without one (packaged exe outside the repo), the legacy next-to-the-original behavior remains as the fallback. `save_edited_image` creates the export directory on demand. `/edited/` is gitignored alongside `decoded-cache/` and `local-edits/`, and the existing decisions.md entry already flags all repo-local runtime directories as needing a `%LOCALAPPDATA%` move before distribution — `edited/` joins that list.

Naming stays `<stem>_edited.<ext>` with overwrite-on-re-export: predictable, and identical to the prior same-directory semantics for a given source. Cross-source stem collisions (two folders both containing `DSC09218.ARW`) overwrite each other's export; accepted for now (solo user, occasional exports) and revisitable with provenance metadata if it ever bites.

The status bar already reports `Saved: <path>` from `Message::SaveCompleted`, so the new location is visible to the user on every save.

## Tests

Repo-rooted expectation tests for the three filename shapes (extension, no extension, RAW→png), a no-repo-root fallback test (new `with_test_photo_repo_root_absent` override helper in `repo.rs`), an export-directory-creation test, and all pre-existing pixel-correctness save tests wrapped in repo-root overrides so test exports stay inside their tempdirs instead of polluting the real repo.
