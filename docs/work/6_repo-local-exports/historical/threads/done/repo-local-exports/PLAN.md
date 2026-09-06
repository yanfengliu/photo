# Implementation plan: repo-local-exports

Target version: 0.2.3. Small change; one step, TDD.

1. Failing tests first: repo-rooted expectations for the three `edited_save_path` filename shapes (extension / no extension / RAW→png), a no-repo-root fallback test (requires a new `with_test_photo_repo_root_absent` override in `repo.rs`), and an export-directory-creation test through `save_edited_image`. Wrap every pre-existing file-writing save test in a repo-root override so suite runs cannot write into the real repo.
2. Implement: `edited_save_path` resolves `repo::photo_repo_root()` → `<repo>/edited/<name>`, else legacy `with_file_name`; `save_edited_image` runs `create_dir_all` on the target parent; add `/edited/` to `.gitignore`.
3. Gates, review (rode along with library-offline-edits iteration 2), docs (changelog 0.2.3, devlog, summary, ARCHITECTURE save-flow + overview, decisions row, drift-log row), version bump, commit.
