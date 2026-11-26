# Review 4: implementation

## Target

Repository photo, commit `66c9fcf216c3224901c2bac75c1656db2d1005c5` (branch `restart`): the repairs of review 3's findings and the small additions made beside them, as the diff `58dcdb9..66c9fcf` over `crates/export` (one lens) and over `crates/engine`, `crates/catalog` and `crates/core` (a second lens); 59 files, 5,950 lines added and 783 removed in all.

The lane read the builders' shared worktree. It was clean at that commit when the lane started. While the second lens ran, the coordinator edited seven files outside the reviewed crates (README, the changelog, the architecture documents, the harness guide and the version line of the root `Cargo.toml`); the lens noticed the README and guide changes and excluded them. The reviewed crates did not change during the run.

## Reviewers and coverage

- **Codex lane.** codex-cli 0.171.0, model `gpt-6-astra`, `model_reasoning_effort=xhigh`, read-only sandbox, `--ephemeral`, two chained lenses from the run folder `fleet/tmp/review-runs/photo-2026-11-25-round3-repairs-codex`: export (429 seconds), then engine, catalog and core (358 seconds). Neither log has a failed command. Each prompt held the claims per repair and the diff, and pointed at review 3's record. Both reports cite files and lines, including lines outside the diff's hunks (`crates/export/src/item.rs:17`, `crates/export/tests/export/safety.rs:24`, `crates/catalog/src/text.rs:23`, `crates/engine/src/jobs/mod.rs:309`, `crates/imaging/src/geometry/crop.rs:55`, tempfile's `windows.rs:99`). It ran no test; the scenarios are code traces.
- **Claude lane: not run.** As in review 3, this is the one-lane check of repairs. `crates/export` and `crates/engine` have been read by one reviewer family only.

## Reports

Each report is copied from the lane's output file by command. Two absolute path prefixes that contain the owner's local user name are removed from its links: the worktree's, and the cargo registry's.

### Codex lane, photo-export

I would not close these repairs. I reviewed `66c9fcf216c3224901c2bac75c1656db2d1005c5` statically, including the prior dispositions. No files changed and no cargo commands ran. The scenarios below are code traces, not executed reproductions.

| Finding | Verdict | What remains |
|---|---|---|
| F29 | **PARTLY FIXED** | The original check-then-replace window is removed, but failed capture, changing identities and copy-based recovery still permit loss. |
| F30 | **PARTLY FIXED** | Taking a free name does not overwrite it. Failed-copy cleanup can nevertheless move somebody else’s file and leave it hidden. |
| F33 | **PARTLY FIXED** | Ordinary late folder aliases are covered. Unreadable initial paths and the declared late symbolic-link window remain unsafe. |
| F46 | **PARTLY FIXED** | Fixed where linking or exclusive rename succeeds. With neither operation, the empty/partial final file and crash residue remain. |

With hard links or exclusive rename, I found no replacement of an existing file in the normal Skip/UniqueName placement path. The fallback and recovery paths do not support the broader guarantees.

1. **F29/F33 — High: an unsuccessful initial open silently removes protection.** [guard.rs:95](crates/export/src/place/guard.rs:95) discards every capture error, including permission and sharing errors.

   Start with an existing original `a.tif` temporarily unreadable and supply its already-decoded pixels. That is expressly supported by [item.rs:17](crates/export/src/item.rs:17), **outside the diff hunks**. Capture fails. During rendering, another program restores access and moves that original onto `out/x.jpg`. Its old path now leads nowhere; the exporter has no retained identity. Both later checks return unguarded, and [overwrite.rs:63](crates/export/src/place/overwrite.rs:63) deletes the original after placing the export. This applies to all three filesystem capabilities. A protected file in another folder has the same failure.

   Treating an existing but unreadable guarded file as absent is not a sufficient safety boundary.

2. **F33 — High: the declared late symbolic-link exception still permits replacement of a caller-protected file.** At [overwrite.rs:110](crates/export/src/place/overwrite.rs:110), the last sighting precedes the move; [guard.rs:151](crates/export/src/place/guard.rs:151) subsequently opens paths after that move has broken them.

   Let protected `/catalog/x.jpg` initially be unavailable and `/out/x.jpg` hold a catalogued original. After `look`, restore `/catalog/x.jpg` as a symbolic link to `/out/x.jpg`. The exporter moves `/out/x.jpg` aside. The protected link now dangles, and `names_entry` cannot rescue it because `/catalog` and `/out` are different directories. The original is classified as replaceable and deleted. This works with hard links, exclusive rename or neither.

   The limitation is documented, but it narrows F33’s requested protection rather than completing it.

3. **F29/F33 — High: reading identities afresh does not make the comparison atomic.** [held.rs:50](crates/export/src/place/held.rs:50) reads the two identities separately. [guard.rs:160](crates/export/src/place/guard.rs:160) relies on their inequality to continue toward replacement.

   Under the stated exFAT behavior, another program can retain a writable handle to the original, move the original onto the destination after `look`, then truncate/rewrite it between the two identity reads. The two handles refer to the same file but report different generations of its identity. Repeat that change during the retained-start comparison. The old pathname is gone and the fallback names differ, so identification returns `None`; the exporter deletes the original.

   This deduction depends on the builder’s reported identity changes. I did not reproduce them here. None of the hooks runs between these identity reads.

4. **F29/F33 — High: restoring by copying can discard a concurrent write to the original.** [moves.rs:205](crates/export/src/place/moves.rs:205) stops copying at EOF; [moves.rs:137](crates/export/src/place/moves.rs:137) subsequently deletes the file copied from.

   On a volume with neither move operation, an original arrives after `look`, is moved aside, recognised and copied back. After the copy reaches EOF and syncs, another editor writes through a handle it opened on the original before export began. The exporter checks only the identity of the **copy**, then removes the original’s remaining name. The refusal points to a stale copy; the editor’s completed write disappears when the retained handles close.

   The other editor never accesses an export-generated hidden pathname. Excluding deliberate interference with those names does not exclude this ordinary open-handle case.

5. **F30 — High: failed-copy cleanup can hide an existing file, including an original, under Skip or UniqueName.** The remaining check/move window is at [moves.rs:231](crates/export/src/place/moves.rs:231), followed by the rename at line 235.

   On a full volume with neither move operation:

   - Copying into `x.jpg` fails.
   - Cleanup confirms that `x.jpg` is its created file.
   - Another program renames a valuable file onto `x.jpg`, replacing only the failed export.
   - Cleanup moves that valuable file to `.photo-export-….discard`.
   - Identification detects the substitution, but restoring it requires another copy. Insufficient space makes [put_back](crates/export/src/place/moves.rs:266) stop.

   The valuable file remains only under the hidden name. The returned error still names `x.jpg` through [mod.rs:184](crates/export/src/place/mod.rs:184); its recovery location is only logged. The `GiveBack` hook precedes the check, so the tests cannot inject this substitution in the relevant window.

6. **NEW — High: legitimate exporters on a network share can collide on hidden names.** [moves.rs:49](crates/export/src/place/moves.rs:49) uses only process ID and a process-local counter. Checking vacancy does not reserve the name; [overwrite.rs:113](crates/export/src/place/overwrite.rs:113) performs a replacing rename onto it.

   Two machines can have the same process ID and counter. Their export histories can bring both to `.photo-export-P-2.previous`; both check it before either renames. Export B moves its newly arrived original there. Export A then renames its displaced destination onto the same hidden name, destroying B’s original before B identifies it.

   Both programs used their own generated names. This is not malicious interference. The two-export test shares one atomic counter and cannot produce this collision.

7. **F46 — Medium: an unfinished final file remains visible on the fallback filesystem.** [moves.rs:120](crates/export/src/place/moves.rs:120) creates the final name before filling it.

   A watcher can read zero bytes before `Fill`, or partial bytes during copying. Stop the process there and that final file remains. A later Skip export accepts the occupied name and leaves it untouched. Eliminating the subsequent rename fixes F30’s destructive reservation behavior, but does not eliminate F46’s observed defect.

8. **NEW — Medium: identity errors are mistaken for name collisions, leaving completed exports behind on failure.** [held.rs:56](crates/export/src/place/held.rs:56) converts lookup and comparison errors into `false`; [moves.rs:142](crates/export/src/place/moves.rs:142) then interprets an occupied name as somebody else’s file.

   On Unix, arrange only two spare descriptors when fallback placement begins. Creating the destination and opening the copy source succeed. After copying, opening the destination for comparison succeeds, but `try_clone` at [held.rs:61](crates/export/src/place/held.rs:61) exceeds the descriptor limit. Skip reports `Skipped` despite having created the completed output. UniqueName repeats this, potentially leaving 100 completed outputs before returning an error. No other program has taken any name.

   There is also an earlier cleanup gap: failure of `Held::keep(created)?` at [moves.rs:130](crates/export/src/place/moves.rs:130) leaves the newly created final file without entering cleanup.

9. **NEW — Medium: a refused Windows export can change a protected original’s attributes.** Recovery calls [disk.rs:87](crates/export/src/place/disk.rs:87). The inspected dependency explicitly executes `SetFileAttributesW(..., FILE_ATTRIBUTE_NORMAL)` before moving: [tempfile/windows.rs:99](tempfile-3.27.0/src/file/imp/windows.rs:99).

   On Windows without hard links, let a protected file with hidden/system attributes arrive after the last look. Recovery returns it through this helper and reports refusal, but its original attributes are gone. A failed move instead attempts to mark it temporary. These mutations are avoidable consequences of using a temporary-file helper for a user’s original, not filesystem requirements.

10. **NEW — Low: Overwrite refuses distinct files on case-sensitive volumes.** [guard.rs:168](crates/export/src/place/guard.rs:168) applies the name fallback even after successful identity comparisons establish different files.

    Put protected `IMG.JPG` and an unrelated existing `img.jpg` in the same case-sensitive directory. Export to `img.jpg`. Both identity checks correctly reject a match, but lowercasing at line 183 causes a protected-file refusal after moving the unrelated file aside. This conservative behavior is documented, but it is an observable false refusal. The case test does not require successful overwrite of an already-existing distinct case variant.

The tests do not establish “every interleaving.” [at_every_step](crates/export/src/place/tests/mod.rs:241) injects one action at each reachable hook in a fresh run. It does not explore arbitrary action sequences, operations inside a hook, or every branch of recovery. In particular, removing the finished file is not equivalent to injecting write, sync, metadata, restore and unlink failures.

| Test class | What removal of the repair would reveal—and what can still pass |
|---|---|
| Original/protected arrival at placement steps | The arrival at `SetAside` detects a return to check-then-replace **if that hook remains between check and rename**. It does not exercise failed initial capture, concurrent identity changes or writes during restoration. |
| Skip/UniqueName renamed-file and empty-file failure tests | They can catch reservation-then-rename and zero-length ownership cleanup when the existing hooks straddle those operations. They miss the internal `give_back` check/rename window. |
| Create-only arrival and two cooperating exports | These can pass with the old exclusive reservation followed by replacement: the reservation already prevents the other creator from taking that name. They do not independently prove safe replacement. |
| An unsafe check-then-rename hidden inside `Disk::rename_exclusive` | The injected actors run before the call, not between its internal check and rename. These deterministic tests could all pass with that unsafe adapter. Atomicity here comes from inspecting the underlying primitive. |
| Held-file first-bytes/rename test; guard rewrite test | They distinguish cached identities only on a filesystem where the exercised operation changes identity. On stable-identity APFS/ext4/NTFS, cached identities can pass. The rewrite guard test specifically matters on exFAT. |
| Protected-folder-return test | It can still pass with late **file identity** lookups removed: its directory alias plus identical basename is rescued by `names_entry`. It does not prove the general late-path identity requirement. |
| Public API movement tests | Their watcher acts when `.part` appears, during rendering: [safety.rs:24](crates/export/tests/export/safety.rs:24), also outside the diff hunks. A final identity check followed by an unsafe rename can still pass them. |
| Retry-limit tests | They distinguish removal of the 100-attempt and four-round limits. |
| Template tests | The empty-token expectations distinguish removal of separator handling. They say nothing about placement safety. |
| F46 | No test inspects the final file during copying or restarts after process termination. Successful whole-file comparisons pass while F46 remains. |

The declared boundaries need separate treatment:

- **Interference with hidden names:** fair for deliberate tampering with an exclusively owned namespace; not fair for the ordinary cross-machine collision in finding 6. It must not silently include writes through handles opened on originals.
- **Replacing unguarded arrivals under Overwrite:** a fair policy boundary.
- **Paths that become protective only later:** fair after replacement has completed. The pre-move symbolic-link case in finding 2 and swallowed unreadable captures are remaining protection gaps.
- **Unswept crash residue — F29/F33, Medium:** after [overwrite.rs:113](crates/export/src/place/overwrite.rs:113), kill the process before restoration. The original remains solely under `.previous`, with no returned error or recovery mechanism. Leaving an abandoned export `.part` is reasonable; leaving an original undiscoverable by the application is a material recovery defect.
- **Failed restoration:** inability to write after a disconnect is an unavoidable limit. Taking a foreign file out of view during Skip cleanup, requiring enough free space to duplicate an original, and reporting only its former path are avoidable exposures.
- **Copy-based restoration and Windows attribute changes:** these are materially weaker than preserving the original. Findings 4 and 9 show consequences beyond a harmless identity change.
- **Unsynced replacement — NEW, Medium:** [overwrite.rs:63](crates/export/src/place/overwrite.rs:63) deletes the previous file without first making a linked/renamed export durable. Power loss can leave neither usable version. Hardware guarantees can reasonably be excluded; omitting available synchronization is an implementation limitation that matters on removable storage. The fallback’s file sync also does not establish directory-entry durability; that requires separate treatment on systems with those semantics. [fsync documentation](https://man7.org/linux/man-pages/man2/fsync.2.html)

I found no additional ordinary retry loop without a bound, nor a path to an empty final stem. The 100-attempt limit does not bound all searching: unchanged [naming.rs:80](crates/export/src/naming.rs:80) can scan a very large occupied range. I did not establish a Windows failure caused solely by retaining the shared handles; Windows runtime behavior remains untested in this review.
### Codex lane, photo-engine, photo-catalog and photo-core

F33, F49 and F51 remain **PARTLY FIXED**. I reviewed `66c9fcf216c3224901c2bac75c1656db2d1005c5` statically and checked the crop arithmetic separately. No cargo commands ran and no repository files changed. The unrelated working-tree changes in README and the harness guide were excluded.

| Finding | Verdict |
|---|---|
| F33 | **PARTLY FIXED** — the unavailable-folder case is covered for matching names; Unicode filename equivalence still allows originals to escape protection. |
| F35, test remark | **FIXED** — separate tests now distinguish removal of the ticket check and removal of the lock. |
| F39, test remark | **FIXED** — the new test establishes a finished, unrecorded copy before drop. |
| F43 | **FIXED** — cancellation is checked during enumeration and before sorting. An individual filesystem call or an already-started sort remains non-interruptible. |
| F45, first part | **FIXED** — offline cache hits retain their source stamp. The new global recheck blockage described below can still prevent subsequent checks. |
| F48 | **FIXED** — cancellation or failure of a newer save no longer suppresses the older save. |
| F49 | **PARTLY FIXED** — catalog-wide failures stop the per-photo fallback, but the claimed aggregate two-second lock budget is not enforced. |
| F50 | **FIXED** — deferred preset requests are issued after decoding, including without a canvas view. |
| F51 | **PARTLY FIXED** — worker occupancy is limited, but the engine’s waiting list can still accumulate duplicate entries after stamp eviction. |

The remaining defects and addition failures are:

1. **F33 — High: equivalent Unicode filenames can bypass protection.**  
   [photos/mod.rs:290](crates/catalog/src/photos/mod.rs:290) filters through `text::fold`, which explicitly does no normalization at [text.rs:23](crates/catalog/src/text.rs:23), outside the diff. Catalog an original named `cafe\u{301}.jpg` through the unavailable directory alias from F33. Export another photo as `CAFÉ.jpg`, then reconnect the original’s drive before the worker runs. The two folded strings differ, so the original is absent from `protected`; APFS resolves the destination to that original, which can consequently be replaced. APFS treats normalization variants as the same filename. [Apple’s filesystem documentation](https://developer.apple.com/library/archive/documentation/FileManagement/Conceptual/APFS_Guide/FAQ/FAQ.html) confirms this behavior. The existing guard’s lowercase-only comparison also misses these variants.

2. **NEW — Medium: consecutive Undo commands can discard earlier redo entries without an intervening edit.**  
   [commands.rs:195](crates/engine/src/commands.rs:195) decides whether to commit from whichever historical command is currently on top. Commit contrast on A, rate B, then begin an uncommitted exposure edit on A. Undo first removes B’s rating and leaves A’s draft open. Undo again commits A’s exposure, clearing B’s redo entry through [undo.rs:272](crates/engine/src/undo.rs:272), and immediately undoes exposure. B’s rating can no longer be redone, although no edit occurred between those two Undo calls. The second `undo_label()` names contrast, while the operation actually undoes exposure. Similarly, `redo_label()` can advertise a command that `redo()` merely discards when it commits the draft. The added tests cover each branch separately, not this mixed sequence.

3. **F49 — Medium: the same remaining timeout is reused across multiple transactions.**  
   [import/run.rs:322](crates/engine/src/import/run.rs:322) sets the timeout once before `record`; [record.rs:41](crates/engine/src/import/record.rs:41) then performs fallback inserts without reducing it. Suppose another connection releases its lock after 1.5 seconds, allowing the batch to run, but the batch encounters a duplicate record and rolls back. That connection reacquires the lock before the first fallback insert. The insert can wait nearly another two seconds. Repeated successful acquisitions between fallback transactions can extend this further. Catalog-wide error classification does not help while each acquisition succeeds before its reused timeout expires.

4. **NEW — Medium: a stalled worker prevents the open edit from reaching the closing save.**  
   [closing.rs:33](crates/engine/src/engine/closing.rs:33) joins workers before starting the deadline or committing the draft. [jobs/mod.rs:309](crates/engine/src/jobs/mod.rs:309), outside the diff, performs an unconditional join. With an exposure edit open and an unrelated worker blocked in filesystem metadata on a stalled share, Quit never reaches `keep_open_edit`, even when the catalog is healthy and local. Force-quitting then loses the draft. The two-second patience does not bound this wait.

5. **NEW, from F51 — Medium: one stalled source blocks all preview rechecks, including completed observations.**  
   [recheck.rs:75](crates/engine/src/previews/recheck.rs:75) collects every result before delivering any; [recheck.rs:145](crates/engine/src/previews/recheck.rs:145) prevents another batch while this one runs. Let a batch observe changed local file A, then block on network file B. A’s already-completed observation is never applied, and subsequent local file C is never checked. Their cached previews can remain stale indefinitely. This avoids the old exhaustion of every worker, so it improves general responsiveness with multiple workers, but it introduces a global freshness dependency on the slowest source.

6. **F51 — Medium: stamp pruning permits duplicate queued rechecks and unbounded waiting-list growth.**  
   [recheck.rs:103](crates/engine/src/previews/recheck.rs:103) removes watches for evicted previews without removing their entries from `due`. Hold one recheck on a stalled drive, then browse enough photos to cross the 65,536-stamp threshold. Revisit an evicted photo whose old recheck remains in `due`: its newly loaded watch has no outstanding-check flag, so another due entry is appended at line 134. Repeating this traversal grows the list beyond one entry per photo, even with a fixed catalog. The three-photo test cannot exercise this path.

7. **NEW — Medium: repeated export names multiply protected-path storage.**  
   [photos/mod.rs:311](crates/catalog/src/photos/mod.rs:311) clones the entire matching path list for every requested name. In a 200,000-photo catalog containing 10,000 `IMG_0001.jpg` originals in different folders, exporting those 10,000 photos produces 100 million owned path entries before excluding each item’s own path. This creates multi-gigabyte storage pressure synchronously, before [export/mod.rs:221](crates/engine/src/export/mod.rs:221) creates the cancellable task.

8. **NEW — Low: turning the normalized crop does not preserve its snapped pixel rectangle.**  
   [turn.rs:75](crates/core/src/develop/turn.rs:75) transforms the continuous rectangle, whereas [imaging/geometry/crop.rs:55](crates/imaging/src/geometry/crop.rs:55), outside the diff, rounds its pixel origin independently. For a 1000-pixel-wide source, `cx = 0.5`, `width = 0.501`, and angle zero, the rendered crop covers source columns **250–750**. Flip horizontally: the normalized rectangle remains centered, but the rendered result covers **249–749**, reversed. One column of content changes. I verified these numbers with float32 setting values and the renderer’s rounding formula.

The additions otherwise have these limits:

- **Virtual copies, removal and sidecar replacement:** the preliminary commits preserve the draft on the inspected successful paths, including removal through a virtual copy’s master. The draft commit and subsequent command are separate transactions; a later command failure leaves the draft committed as its own history step.
- **Closing:** it attempts the draft before recording finished imports, once worker shutdown finishes. If the lock consumes the patience, the draft survives only in the error log’s description; no recoverable settings payload is saved. Catalog-wide import failures retain their copied files and log paths. Record-specific refusals still call `forget_copy` during closing at [import/run.rs:338](crates/engine/src/import/run.rs:338), so the “left on disk” claim applies only to stranded catalog-wide failures.
- **Catalog APIs:** the error classification and timeout setter implement their stated mechanisms without a schema change. `files_named` performs one scan over the master filenames per export batch—approximately 200,000 folds for 200,000 masters, even for one exported photo—plus result construction and duplication. There is no matching expression index. I did not measure elapsed time.
- **Turn and Library rotate/flip:** the continuous crop geometry, ratio exchange and straighten-angle sign are consistent across the eight orientations. The pixel-rounding counterexample prevents an unconditional same-content verdict. The rendered tests do not establish edge-touching behavior across all orientations. Masks and healing spots remain attached because their coordinates are in the source frame. The unchanged frame-relative settings are the perspective transform’s vertical/horizontal correction, rotation, aspect, scale and X/Y offsets, deliberately excluded by the documented policy. Film grain is generated in output coordinates, so its individual grains also do not rotate like pixels on a print.

The test conclusions are static mutation assessments, not executed results:

- **F35/F48:** `the_other_saves_of_a_sidecar_wait_while_one_writes` checks the held lock deterministically; `a_newer_save_that_finishes_first_is_not_overwritten_by_the_older_one` checks the ticket condition. The cancelled-newest and failed-newest cases distinguish the old claimed-ticket behavior. The retained `overlapping_saves_of_one_sidecar_end_with_the_last_values` remains scheduling-dependent.
- **F39:** the new `closing_the_app_during_a_copy_import_leaves_no_file_without_a_record` would fail without the final ready-slot recording. It already applies the copy-completion event before drop, so it would still pass if only the closing event drain were removed. The renamed `closing_the_app_part_way_through_a_copy_import_leaves_no_file_without_a_record` remains timing-dependent and can pass without the original repair.
- **F49:** `a_batch_the_catalog_cannot_take_is_reported_and_its_copies_are_removed` would pass with the old per-photo fallback: its timeout is zero and every insert fails. The two call-count tests in `import/record.rs` distinguish the repaired fallback. Both closing lock tests use zero patience and allow nearly four seconds; neither proves a shared two-second budget. Their wall-clock assertions also include worker shutdown. `a_change_to_a_locked_catalog_fails_at_once_when_told_not_to_wait_and_says_it_was_the_catalog` never measures elapsed time.
- **F43:** the iterator-driven cancellation tests distinguish the enumeration and pre-sort checks without scheduling dependence. The sorting and uninterrupted-scan tests would pass without cancellation support.
- **F50:** both preset tests distinguish their respective repairs. Both new Before tests would pass with the F50 changes removed because those request paths already existed.
- **F45/F51:** the offline-preview tests distinguish preservation of the disk stamp. `while_a_look_is_out_asking_again_sends_no_other` distinguishes repeated submission for its three retained watches, but releases the worker before checking freshness and never exercises eviction.
- **Turn/history:** the rendered turn tests distinguish leaving the crop unchanged, but use an interior crop at 7°. The core corner test bypasses pixel snapping. The open-edit tests do not check menu labels or the cross-photo Undo sequence in finding 2.
## Findings and disposition

The lane's verdicts on review 3's findings. Fixed: F35's and F39's test remarks, F43, F45 (first part), F48, F50. Partly fixed: F29, F30, F33, F46, F49, F51. What remains of each is given a new id below, F52 to F71, in the order the reports list them.

How the remaining export cases were judged. Review 2 found an original destroyed if it was moved during a render, which is seconds long and something a person can do. Review 3 narrowed that to the instant between a check and a rename. What is left now needs another program to make one particular move inside that instant, or inside a single comparison. The crate's "Safety of the originals" section says where the guarantee ends. A case is accepted here when closing it is cheap and removes a class; it is rejected, with its reason, when it needs a program acting on purpose in that instant.

| ID | Finding | Disposition and reason | Repair or follow-up |
|---|---|---|---|
| F52 | Export 1 (of F29 and F33, high): a source or protected file that cannot be opened when the export starts is treated as absent, so it is unguarded if it later turns up at the destination | Accepted: an existing file whose identity cannot be read is not the same as no file | photo-export builder: under the overwrite policy the export is refused before rendering |
| F53 | Export 2 (of F33, high): a symbolic link created at a catalogued path, pointing at the export's destination, between the last look and the move, leaves that original unrecognised | Rejected as outside the guarantee: it needs another program to create that link inside the instant, and the catalog's paths are compared by identity both before and after the move | None; stated in the crate's safety section |
| F54 | Export 3 (of F29 and F33, high): two identity reads are not one atomic step, so a file rewritten between them on exFAT can read as two files | Rejected: it needs a program that moves the original onto the destination and rewrites it between two consecutive system calls | None |
| F55 | Export 4 (of F29 and F33, high): a write another program makes through its own open handle, after a copy back has finished, is lost | Rejected: it needs the original to arrive at the destination inside the instant, on a volume with neither safe move, and an editor writing to it at that moment | None |
| F56 | Export 5 (of F30, high): on a full volume with neither safe move, the cleanup after a failed copy can move aside a file another program has just renamed onto the name, and leave it under a hidden name | Accepted as a class: nothing that is not this export's own file may be left under a hidden name by a path that returns | photo-export builder: such a file is given a visible name beside its old one and the error carries the path |
| F57 | Export 6 (new, high): hidden names are built from the process id and a counter, so two machines exporting into one shared folder can pick the same one, and the set-aside rename replaces | Accepted | photo-export builder: random bits in hidden names; set aside by a move that cannot replace where there is one |
| F58 | Export 7 (of F46, medium): where the file system has neither safe move, an unfinished file shows under its final name while it is filled, and stays if the process stops | Noted, not repaired: on such a volume the choice is between this and renaming onto a name, which is what F30 forbade. The changelog and the crate say so | None |
| F59 | Export 8 (new, medium): an error while reading or comparing an identity is taken for "another file has the name", so skip can report skipped for a file it wrote and unique-name can leave finished files behind | Accepted | photo-export builder: such an error is returned as an error and what was created is removed |
| F60 | Export 9 (new, medium): on Windows without hard links, putting a protected file back clears its hidden, system and read-only attributes | Noted, not repaired: it is the put-back path of a refused export on FAT or exFAT under Windows only, and the standard library has no way to carry those attributes across | The plan's scope, under Deferred |
| F61 | Export 10 (new, low): on a case-sensitive volume, a different file whose name differs from a protected one only in letter case is refused | Noted: conservative on purpose | None |
| F62 | Export, declared boundaries (medium): a process killed between setting an arriving original aside and putting it back leaves it under a hidden name, and nothing sweeps for such files | Noted: it is F53's instant plus a crash; a sweep for leftover hidden files is deferred | The plan's scope, under Deferred |
| F63 | Export, declared boundaries (new, medium): the previous file is removed before the new one is synced, so power loss can leave neither | Rejected: decision 12 in `docs/architecture/decisions.md`; an export can always be made again from the catalog | None |
| F64 | Engine 1 (of F33, high): file names the file system treats as one name but that are encoded differently (`cafe` with a combining accent, and `CAFÉ`) are not matched by `Catalog::files_named` or by the guard's lower-case comparison | Accepted. The stored `file_name_key` column is not changed | photo-engine builder: both compare on a canonical-normalised, case-folded key |
| F65 | Engine 2 (new, medium): with an edit open on one photo, two undos in a row whose first undoes a command on another photo drop that command's redo entry, and the menu's label names the wrong step | Noted, not repaired: not reachable through the interface, which records a tool's work before undo and redo, and a drag cannot have two undos pressed inside it. It is a quirk of calling the engine directly | None |
| F66 | Engine 3 (of F49, medium): the closing patience is set once and reused by each catalog write, so with a lock that comes and goes closing can wait longer than its two seconds | Accepted | photo-engine builder: one deadline for the whole of closing |
| F67 | Engine 4 (new, medium): closing joins the workers before it commits the open edit, so a worker stuck on a stalled drive keeps the edit from being saved | Accepted | photo-engine builder: the open edit is committed first |
| F68 | Engine 5 (new, from F51's repair, medium): the single recheck worker delivers nothing until its whole batch is done, so one stalled source holds up every other recheck | Noted, not repaired: rechecking is a freshness check, and a drive that hangs suspends it rather than the workers that decode and render, which is the trade F51 asked for | None |
| F69 | Engine 6 (of F51, medium): a preview whose stamp is evicted stays in the list waiting for a look, so the list can hold it twice | Accepted | photo-engine builder |
| F70 | Engine 7 (new, medium): the list of protected paths is cloned per photo, so exporting ten thousand photos that all share one file name builds a hundred million entries | Noted, not repaired: camera file names repeat every ten thousand frames, so a name is shared by tens of photos in a large catalog, not thousands | None |
| F71 | Engine 8 (new, low): after a mirror, pixel snapping can move the crop by one column | Noted: rounding | None |

Remarks on tests that carry no finding of their own: the lane lists which tests would still pass with their repair removed (among them the two new Before-view tests, which confirm behaviour that already held, as their builder said) and which depend on scheduling. `at_every_step` injects one action per run at each hook, not sequences of actions; the report's table of what each class of placement test does and does not prove stands as written.

## Verification

The eight repairs landed on `restart` on 2026-11-26: F64 `ed23b25`, F67 `2a8889b`, F66 `3dfd2b5`, F69 `061ed95`, F52 `adb50ea`, F59 in `3dfd2b5` (two builders shared one git index, so F59's files went into the F66 commit; `106a603` is an empty commit that carries F59's message), F56 `a0c992f`, F57 `86564d4` and `5afc614`; the crate's safety section was rewritten in `0f504d3`. Each builder reported a test that failed before its repair, except `held::tests::a_path_that_cannot_be_opened_is_not_another_file`, which could not compile against the old signature. The export builder ran the export tests on APFS, FAT32 and exFAT images (78 unit and 36 integration tests each); the tests that need a file that cannot be opened ran on APFS only, because FAT and exFAT ignore permissions. F66 as built gives closing one deadline for all its catalog writes but does not count the time spent waiting for workers to stop, so a slow but healthy shutdown still records finished copies; the coordinator accepted that. F64's key is used by the catalog and the engine's guard; the export crate's own name fallback (`place/guard.rs`) still compares lower-cased names, which matters only for a protected path that leads nowhere.

The coordinator read `crates/export/src/place/aside.rs` (setting aside, hidden names, putting back) and the closing order. The four gates on the merged tree and CI on macOS and Windows are recorded in the plan's outcome when the release lands. No further lane was run: the eight are small and each closes a class named above.

## Round outcome

Closed on 2026-11-26 with the eight repairs landed. As first written, open until they landed. The class review 2 opened, an export replacing a file it must not, is closed where the file system has a safe move; on FAT and exFAT what remains is listed and bounded. Of twenty new items, eight are accepted, seven are noted with reasons, three are rejected as outside the guarantee, one is rejected by an existing decision, and one is a low-severity rounding note.
