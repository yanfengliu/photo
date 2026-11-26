# Review 2: implementation

## Target

Repository photo, commit `a9cfdc59c7bb80114dd802984a0fdc603b1c458e` (branch `restart`), the crates `crates/export` (photo-export) and `crates/engine` (photo-engine) in full. Both are entirely new since the reviewed revisions of rounds 0 and 1, so each lane was asked to review the whole crate, source and tests, rather than a diff. The lanes read a detached worktree checked out at that commit, clean before and after the Codex lane.

These two crates are the remaining high-risk ones: the exporter writes image files into folders that hold originals, and the engine is the application's concurrency, its file copies during import, its sidecar saves, its export jobs and its undo stack.

## Reviewers and coverage

- **Codex lane.** codex-cli 0.171.0, model `gpt-6-astra`, `model_reasoning_effort=xhigh`, read-only sandbox, `--ephemeral`, two chained lenses (export, then engine) from the run folder `fleet/tmp/review-runs/photo-2026-11-20-engine-codex`. Both reports cite files and lines. It ran no tests and no filesystem race reproductions, which the read-only sandbox does not allow.
- **Claude lane: abstained.** Launched on 2026-11-20 with the runbook's command and limits (claude 2.1.329, `claude-opus-5-5[1m]`, `--effort max`) from `fleet/tmp/review-runs/photo-2026-11-20-engine-claude`. It split the work among four subagents; all four were still reading when the account's weekly usage limit stopped them, and the log's one result event is the limit message, which is an error. By the runbook's keep rule the lane abstains. Its log holds no finding from before the cut-off (each subagent's recovered text is a single sentence of intent). The CLI reported $97.35 for the attempt. It has not been re-run: that limit is shared with the builders, and the owner waited four days for it to reset.
- **Missing coverage.** One lane only for these two crates, and no executed reproduction of the races it describes. The owner has been told.

## Reports

Each report is copied from the lane's output file by command; the absolute path prefix of the review worktree, which contains the owner's local user name, is removed from its links.

### Codex lane, photo-export

Found four defects at `a9cfdc59c7bb80114dd802984a0fdc603b1c458e`.

1. **High — Moving an original during rendering can destroy it.** [place.rs:80](crates/export/src/place.rs:80)  
   Export decoded `a.tif` to `b.tif` with `Overwrite`. While rendering, move `a.tif` to `b.tif`. Placement checks against the old source pathname; `same_file` returns `false` when that pathname no longer exists (lines 162–174). Line 83 then replaces `b.tif`, destroying the original’s only remaining copy. The exporter retains no original file identity across rendering.

2. **High — Unique-name and skip policies can overwrite files on filesystems without hard links.** [place.rs:125](crates/export/src/place.rs:125)  
   On an exFAT destination, two exports targeting `photo.jpg` can both fail `hard_link`, then both observe the destination as absent. Their subsequent `rename` calls both succeed, with the second replacing the first export. An unrelated file created during that interval is equally vulnerable. Both policies use this fallback; its existence check and replacement are separate operations. Rust explicitly documents that [`rename` replaces an existing destination](https://doc.rust-lang.org/std/fs/fn.rename.html).

3. **Medium — Windows hard-link aliases bypass original protection.** [place.rs:172](crates/export/src/place.rs:172)  
   Create `linked.tif` as an NTFS hard link to `photo.tif`, then export `photo.tif` to `linked.tif` with `Overwrite`. Windows canonicalization returns different paths for these names, so their lowercase strings compare unequal. Placement replaces `linked.tif` instead of returning `WouldOverwriteSource`; the other link retains the original bytes. This contradicts the existing [hard-link safety test’s assertion](crates/export/tests/export/safety.rs:68). Microsoft confirms that [the underlying API preserves the name used to open a hard link](https://devblogs.microsoft.com/oldnewthing/20160406-00/?p=93271%2F).

4. **Low — Some reserved Windows device names survive sanitization.** [naming.rs:18](crates/export/src/naming.rs:18)  
   With destination `C:\exports`, template `COM¹`, JPEG format and `Overwrite`, sanitization leaves the stem unchanged and plans `COM¹.jpg`. The reserved-name list contains only ordinary digits, so placement fails instead of producing a safe filename. Windows also reserves `COM¹`, `COM²`, `COM³` and their `LPT` counterparts, including names with extensions. [Microsoft naming rules](https://learn.microsoft.com/en-gb/windows/win32/fileio/naming-a-file)

Reviewed the entire crate and its tests serially, including sizing, encoding, strip boundaries, ICC profiles, metadata, sharpening and watermarks. Findings were checked against control flow and filesystem API behavior. Cargo tests and filesystem race reproductions were not run in the read-only sandbox. No files changed.
### Codex lane, photo-engine

Reviewed the whole engine crate and its tests at `a9cfdc59c7bb80114dd802984a0fdc603b1c458e`. I found 13 issues. These are statically traced findings; I did not run tests or modify files in the read-only sandbox.

1. **High — the export guard can allow overwriting a catalogued original through a directory alias.** [export/guard.rs:47](crates/engine/src/export/guard.rs:47) filters catalog folders by their unresolved basename before canonicalizing them. Import `/photos/link/IMG.jpg`, where `link` points to `/disk/actual`, then export a different photo to `/disk/actual/IMG.jpg` with `Overwrite`. The catalog folder is skipped because `link != actual`; both guard checks pass. The export crate protects only its current source, so it replaces the other original. The “through_links” test at line 129 creates no symbolic link and does not exercise this case.

2. **Medium — an unrelated import silently discards an uncommitted develop edit.** [engine.rs:433](crates/engine/src/engine.rs:433), [develop/load.rs:138](crates/engine/src/develop/load.rs:138). Open A, adjust exposure without committing, then finish importing B through `poll()`. Import insertion emits `Change::Photos`, which reloads A. Because A’s in-memory settings differ from its committed settings, the reload replaces both with the catalog value. The adjustment disappears without an undo entry. Creating virtual copies or removing another photo can trigger the same broad refresh.

3. **Medium — overlapping metadata saves can leave the older snapshot on disk.** [sidecars.rs:75](crates/engine/src/sidecars.rs:75). With two workers, save A at rating 1, change it to rating 2, and save again. Each call captures its own metadata and submits an independent background job without per-path ordering. If the second write finishes first, the older job subsequently writes rating 1. Both tasks report success while catalog and sidecar disagree. XMP’s merge retry preserves unrelated properties, but explicitly supplied properties take the job’s captured values.

4. **Medium — pending Auto Tone can overwrite a newer contrast adjustment.** [quick.rs:134](crates/engine/src/quick.rs:134), [quick.rs:211](crates/engine/src/quick.rs:211). Call `quick_auto_tone(A)` with contrast 0, then `quick_adjust(A, Contrast, 25)` before polling completion. Auto Tone computes from the captured settings and returns contrast 0—the develop implementation deliberately preserves its input contrast. Completion replaces the current entire `tone`, reverting contrast to 0 despite that later adjustment. There is no settings-version check.

5. **Medium — completing an automatic job commits an unrelated slider drag.** [quick.rs:205](crates/engine/src/quick.rs:205), [develop/batch.rs:49](crates/engine/src/develop/batch.rs:49). Start automatic adjustment for A, open B, and begin dragging B’s exposure. When A completes during `poll()`, `editable_settings(A)` unconditionally calls `commit_edit()` for B. Continuing and releasing the drag creates another B history step. Consequently, one undo returns only to the intermediate exposure, rather than the state before the drag.

6. **Medium — undoing keyword creation removes metadata from a later import.** [keywording.rs:323](crates/engine/src/keywording.rs:323), [undo/tree.rs:146](crates/engine/src/undo/tree.rs:146). Add a previously nonexistent keyword `Trip` to A, then import B whose metadata also contains `Trip`. Imports neither enter nor clear the undo stack. Calling `undo()` reverses A’s assignment and deletes the keyword created by that command. Catalog keyword deletion removes **all** assignments, including B’s independently imported metadata; B itself remains imported.

7. **Medium — dropping the engine can leave copied files without catalog records.** [import/run.rs:189](crates/engine/src/import/run.rs:189), [engine.rs:439](crates/engine/src/engine.rs:439). Let a copy finish publishing its destination, then drop the engine before its completion is polled or its ready batch is inserted. Drop cancels work and joins workers but neither applies these results nor removes their published copies. `Copied` has no cleanup ownership. The destination survives without a catalog row. The cancellation test calls `settle()` after cancellation, so it does not establish the same guarantee during shutdown.

8. **Medium — importing newer-format sidecar settings silently turns them into editable defaults.** [import/files.rs:46](crates/engine/src/import/files.rs:46), [import/files.rs:75](crates/engine/src/import/files.rs:75). Import a photo whose sidecar contains private settings with format 99. XMP returns `develop: None` and `unreadable_settings_format: Some(99)`. Import ignores the unreadable-format marker and reports no sidecar problem. Catalog insertion consequently stores no settings, which subsequently resolve to editable defaults; previews and exports proceed with those defaults instead of enforcing newer-settings protection. The original sidecar remains protected by XMP, but the imported catalog loses that protection and warning.

9. **Medium — selection-wide keyword replacement performs one transaction per photo.** [keywording.rs:350](crates/engine/src/keywording.rs:350). `set_keywords()` over 10,000 photos calls `Catalog::set_photo_keywords()` 10,000 times synchronously. Each call opens and commits its own immediate transaction. Undo and redo likewise execute individual keyword-list writes. This blocks the engine thread for a transaction loop instead of one batch and permits partial persistence if interrupted. Failure recovery is compensating writes; rollback errors are only logged.

10. **Medium — navigating away does not cancel obsolete source decodes.** [develop/load.rs:216](crates/engine/src/develop/load.rs:216), [jobs/queue.rs:83](crates/engine/src/jobs/queue.rs:83). Rapidly open many different large raster photos with one worker. Every open submits an unkeyed, uncancellable urgent decode, and urgent jobs execute FIFO. Closing each session clears render requests but leaves these decodes queued. The last selected photo waits for every obsolete decode; generation checking discards results only afterward. Queued completion closures also retain decoded sources outside the cache’s eviction control until polled.

11. **Medium — scan cancellation does not stop processing the current directory.** [import/scan.rs:183](crates/engine/src/import/scan.rs:183). Scan one directory containing many thousands of photos, then cancel after the first chunk. Cancellation is checked only before entering a directory; the entry loop continues reading metadata and posting chunks for the entire directory. `post()` also lacks a cancellation check. `wait_idle()` therefore waits for that work, and engine destruction blocks while joining it. With one worker, a replacement scan waits behind the cancelled scan.

12. **Medium — late renders can restore a disabled before view or identify the wrong preset.** [develop/frames.rs:140](crates/engine/src/develop/frames.rs:140). Enable Before, disable it before its render is delivered, then poll. Disabling clears `before_frame` but does not cancel the pending request; delivery unconditionally fills it again. Similarly, let preset A’s result queue, switch to B, then poll: A’s pixels are stored under B’s current key. Delivery checks the photo-session generation, but not the requested before choice or preset identity.

13. **Medium — a transient preview failure becomes permanent for that develop version.** [previews/mod.rs:147](crates/engine/src/previews/mod.rs:147). Request a thumbnail while its original is unavailable and no disk preview exists; wait for failure, then reconnect the drive. Subsequent requests immediately return the cached failure without checking the source or scheduling work. There is no retry/invalidation path for source recovery; changing develop version or restarting is required. Successful memory entries similarly bypass source-stamp validation, so replacing an original can leave its thumbnail stale while a fresh export uses the new file.
## Findings and disposition

Ids continue the work unit's sequence. The builders were sent these as E1 to E4 (export) and G1 to G13 (engine); both names are given.

| ID | Finding | Disposition and reason | Repair or follow-up |
|---|---|---|---|
| F29 (E1) | An original moved onto the destination during rendering is overwritten, because the source's identity is not kept across the render (Codex, export 1, high) | Accepted | Sent to the photo-export builder: capture the source's file identity at the start and refuse any destination that has it |
| F30 (E2) | On a filesystem without hard links the unique-name and skip policies check, then rename, and can replace a file (Codex, export 2, high) | Accepted | Sent to the photo-export builder: reserve the final name atomically |
| F31 (E3) | On Windows two hard links to one file canonicalise to different paths, so the guard misses them; a test asserts the opposite (Codex, export 3, medium) | Accepted | Sent to the photo-export builder: compare file identity, with the `same-file` crate |
| F32 (E4) | Reserved Windows device names with superscript digits survive sanitising (Codex, export 4, low) | Accepted | Sent to the photo-export builder |
| F33 (G1) | The export guard skips a catalog folder reached through a symbolic link, so an export can overwrite another catalogued original (Codex, engine 1, high) | Accepted | Sent to the photo-engine builder: compare file identity; test with a real linked folder |
| F34 (G2) | A finished import reloads the open photo and discards its uncommitted edit (Codex, engine 2) | Accepted | Sent to the photo-engine builder |
| F35 (G3) | Overlapping sidecar saves of one photo can leave the older values on disk (Codex, engine 3) | Accepted | Sent to the photo-engine builder: the last request wins per photo |
| F36 (G4) | A pending Auto Tone overwrites a newer adjustment (Codex, engine 4) | Accepted | Sent to the photo-engine builder |
| F37 (G5) | Applying an automatic result for one photo commits another photo's drag in progress (Codex, engine 5) | Accepted | Sent to the photo-engine builder |
| F38 (G6) | Undoing the creation of a keyword deletes it from photos imported later with it (Codex, engine 6) | Accepted | Sent to the photo-engine builder |
| F39 (G7) | Dropping the engine can leave copied files without catalog records (Codex, engine 7) | Accepted | Sent to the photo-engine builder |
| F40 (G8) | Importing a sidecar with unreadable private settings silently yields editable defaults (Codex, engine 8) | Accepted | Sent to the photo-engine builder: report it per file |
| F41 (G9) | Selection-wide keyword replacement runs one transaction per photo (Codex, engine 9) | Accepted | Sent to the photo-engine builder, with leave to add one batch call to photo-catalog |
| F42 (G10) | Obsolete source decodes are not cancelled when the user moves on (Codex, engine 10) | Accepted | Sent to the photo-engine builder |
| F43 (G11) | Cancelling a folder scan does not stop the directory being read (Codex, engine 11) | Accepted | Sent to the photo-engine builder |
| F44 (G12) | A late render restores a Before view that was turned off, or lands under another preset's key (Codex, engine 12) | Accepted | Sent to the photo-engine builder |
| F45 (G13) | A preview failure is permanent for that develop version, and memory hits skip the source check (Codex, engine 13) | Accepted | Sent to the photo-engine builder |

## Verification

The repairs landed on `restart` at `2ad1f00` on 2026-11-25: the fixes are in the snapshot commit `a214e70`, and `caf11b4`, `c52186d`, `8b9ef09`, `e304e5a` and `2ad1f00` add or sharpen tests.

What the builders ran at that revision, as they reported it: `cargo test -p photo-export` (42 unit and 32 integration tests pass, 1 ignored), `cargo test -p photo-engine` (52 unit and 88 scenario tests pass, 3 ignored), `cargo test -p photo-catalog` (all pass), and clippy with `-D warnings` and the format check on those three crates, all clean. The workspace-wide format check failed then, only in `crates/ui`, which another builder was editing. The coordinator has not re-run these; the four gates run on the merged tree before it reaches `main`.

The coordinator read the export placement code (`crates/export/src/place.rs`, `lib.rs`) and the engine's export guard (`crates/engine/src/export/guard.rs`, `job.rs`) at `2ad1f00`, and did not read F39's drop path.

The Codex lane's check of the repairs is [review 3](3_implementation.md).

## Round outcome

Closed on 2026-11-25. Seventeen findings from one lane, all accepted, three of them rated high and all three about an export replacing a file it must not. The Claude lane abstained for lack of quota. All seventeen were repaired; review 3 found nine of the repairs complete and eight partial, and six new defects in the repairs, which continue there under the same ids.
