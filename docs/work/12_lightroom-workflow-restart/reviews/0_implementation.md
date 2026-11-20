# Review 0: implementation

## Target

Repository photo, commit `7b3ea0f4e5b70614338f9ffd90fdb52c6a680b8d` (branch `restart-wave1`), the crates `crates/catalog` (photo-catalog) and `crates/xmp` (photo-xmp) in full. Both are entirely new at that revision, so each lane reviewed the whole crate, source and tests, rather than a diff. The lanes read a detached worktree checked out at that commit, which `git status --porcelain --untracked-files=all` showed clean before and after both lanes.

These two crates were reviewed first because they are the high-risk ones finished so far: the catalog is the application's only persistence, and the sidecar writer writes beside the user's original photos.

## Reviewers and coverage

- **Codex lane.** codex-cli 0.171.0 (upgraded from 0.142.5 with the owner's approval on 2026-11-20), model `gpt-6-astra`, `model_reasoning_effort=xhigh`, read-only sandbox, `--ephemeral`, run as two chained lenses (catalog, then xmp) from the run folder `fleet/tmp/review-runs/photo-2026-11-20-wave1`. Both reports cite files and lines; the catalog lens replayed its scenarios against the schema in in-memory SQLite, and the xmp lens ran the crate's 120 existing tests from prebuilt binaries. The pinned model was accepted, so Sol was not needed.
- **Claude lane.** claude 2.1.329 (upgraded from 2.1.196 with the owner's approval; the owner signed in again on 2026-11-20), model `claude-opus-5-5[1m]`, `--effort max`, the runbook's diff-review command and limits, from the separate run folder `fleet/tmp/review-runs/photo-2026-11-20-wave1-claude`. It divided the work among four subagents (catalog storage, catalog queries, XMP XML and files, XMP develop mapping) and re-read every candidate itself. It built and ran nothing, so its findings are verified by reading.
- **Lane validity.** The runbook's keep rule for the Claude lane expects one `modelUsage` key equal to `claude-opus-5-5[1m]`; build 2.1.329 reports the key as `claude-opus-5-5` with `contextWindow: 1000000`, which is the 1M variant. The rule was applied with that substitution: one result event, not an error, non-empty result, one model at a 1,000,000-token context window, empty stderr. Because the runbook proved the command's limits on 2.1.308 only, they were proved again on 2.1.329 first (run folder `photo-2026-11-20-probe`, a throwaway repository): init offered only Task, Bash, Glob, Grep and Read, in `dontAsk` mode, with no MCP servers; every write the lane attempted was refused (a shell redirect, `mkdir`, `node -e` writing a file, `git diff --output`, `git log > file`, and a subagent's `touch`); Write, Edit, WebSearch and ToolSearch were not available; only read-only `git diff`, `git log` and `ls` ran; the tree was unchanged; one model answered. During the review itself the lane read the review tree and dependency sources under `~/.cargo/registry` only, not the Codex run folder. The CLI reported a cost of $27.49 over 89 turns.
- **Missing coverage.** Neither lane ran a filesystem or concurrency reproducer: Codex's sandbox is read-only and the Claude lane's limits forbid building. No review covered the other crates of the restart in this round.

## Reports

Each report below is copied from the lane's output file by command. The only change is that the absolute path prefix of the review worktree, which contains the owner's local user name, is removed from the Codex reports' links, leaving repository-relative paths.

### Codex lane, photo-catalog

Found two **medium-severity** issues at `7b3ea0f`.

1. **Restoring a removal record can reuse a preview-cache version.** [removal.rs:152](crates/catalog/src/removal.rs:152) reinserts the captured photo row, including its old `develop_version`.

   Concrete sequence: edit A → version 1; remove and retain record R; restore R; undo A → version 2; redo A → version 3; remove again; restore the original R → version 1; make different edit B → version 2. Version 2 now identifies both the untouched rendering and B. Caching by `(photo_id, develop_version)`, as the design requires, can display the wrong image.

   Restoration neither preserves a version high-water mark nor rejects the reused record. The repeated-removal test at [tests/removal.rs:254](crates/catalog/tests/removal.rs:254) captures a fresh record every cycle, so it misses this sequence.

2. **Collection and keyword deletion cannot be undone exactly.** [collections.rs:223](crates/catalog/src/collections.rs:223) and [keywords.rs:136](crates/catalog/src/keywords.rs:136) permanently cascade-delete their subtrees and assignments, return only `()`, and have no corresponding restoration API. Recreating them allocates different IDs.

   Concrete consequence: assign keyword K to photo P; remove P, retaining its removal record; delete K; recreate K to undo that deletion; restore P. P silently loses its keyword because [removal.rs:98](crates/catalog/src/removal.rs:98) restores assignments only when the **original keyword ID** survives. Collection memberships have the same problem. This prevents the reversible-command contract in [design.md:162](docs/work/12_lightroom-workflow-restart/design.md:162).

Read all crate source and tests serially. Verified these scenarios by replaying the schema and relevant SQL in in-memory SQLite, including history and foreign-key checks. The Rust suite was not run because it requires filesystem writes. No files changed.
### Codex lane, photo-xmp

Found six defects at `7b3ea0f4e5b70614338f9ffd90fdb52c6a680b8d`.

1. **High — Concurrent saves can erase another application’s changes.** [file.rs:58](crates/xmp/src/file.rs:58), [file.rs:98](crates/xmp/src/file.rs:98). Photo reads sidecar A; another application saves B containing new foreign metadata; Photo then renames its replacement over B and returns success. There is no locking or conflict detection between the read and replacement. The rename prevents a partial file, but the completed foreign update is lost.

2. **Medium — Replacing a list can silently put it in the wrong namespace.** [rdf/edit.rs:82](crates/xmp/src/rdf/edit.rs:82). Let the description inherit `r` as RDF, but use this valid property:
   ```xml
   <dc:subject xmlns:r="urn:foreign">
     <s:Bag xmlns:s="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
       <s:li>old</s:li>
     </s:Bag>
   </dc:subject>
   ```
   Read it, change `keywords` to `["new"]`, then call `to_string`. `set_list` selects `r` from the description’s scope, preserves the property’s shadowing declaration, and inserts `<r:Bag><r:li>new</r:li></r:Bag>`. Those elements now belong to `urn:foreign`; parsing the result returns **no keywords**. The same replacement mechanism handles tone curves.

3. **Medium — Qualified properties are misread, then their foreign metadata is deleted.** [rdf/read.rs:105](crates/xmp/src/rdf/read.rs:105), [rdf/edit.rs:53](crates/xmp/src/rdf/edit.rs:53). For example, with the usual namespace declarations:
   ```xml
   <xmp:Rating>
     <rdf:Description>
       <rdf:value>4</rdf:value>
       <acme:source>editorial review</acme:source>
     </rdf:Description>
   </xmp:Rating>
   ```
   This is a qualified-property form defined by the [XMP specification](https://raw.githubusercontent.com/adobe/XMP-Toolkit-SDK/refs/heads/main/docs/XMPSpecificationPart1.pdf). The reader treats its nested content as `Other`, returning zero stars. Setting the rating to five replaces the entire subtree with plain text, permanently deleting `acme:source`. The reader should understand the qualified value, and updating it should preserve unknown qualifiers.

4. **Medium — Finite crop numbers can produce infinite “sanitised” settings.** [geometry/crop.rs:40](crates/xmp/src/geometry/crop.rs:40). A sidecar containing `HasCrop="True"`, `CropLeft="0"`, `CropTop="0"`, `CropRight="1e40"`, `CropBottom="1e40"` and zero angle passes the finite-`f64` checks. Converting the rectangle to `f32` produces infinity for both centres and dimensions. `sanitize_for` does not validate the rectangle, so `parse` returns those infinities. Writing that result drops the nonfinite corners; its private JSON contains `null` values and cannot restore the rectangle. Reading the written file therefore loses the crop.

5. **Medium — Illegal XML character references are accepted and silently deleted during unrelated edits.** [dom/parse.rs:130](crates/xmp/src/dom/parse.rs:130), [dom/parse.rs:236](crates/xmp/src/dom/parse.rs:236). An XML 1.0 description with `xmp:Rating="1"` and `acme:Note="A&#1;B"` is accepted. I checked quick-xml’s implementation: these decoding functions accept U+0001; they do not enforce XML’s legal-character constraint. Changing the rating rebuilds the start tag, and [dom/text.rs:51](crates/xmp/src/dom/text.rs:51) drops U+0001, changing the foreign value to `AB`. The malformed sidecar should be refused. The malformed-input test checks `&#0;`, which does not expose this gap.

6. **Medium — Namespace validation allows quadratic CPU exhaustion.** [dom/parse.rs:175](crates/xmp/src/dom/parse.rs:175), [dom/parse.rs:195](crates/xmp/src/dom/parse.rs:195). Put 100,000 distinct namespace declarations and matching attributes—`xmlns:p0="urn:0" p0:a="x"`, etc.—on one description. Every attribute searches the declaration vector backwards. The ordinary attribute lookups alone require **5,000,050,000 prefix comparisons**. This shallow, few-megabyte input bypasses the depth limit; there is no attribute or input-size limit to bound the work.

I read the whole crate, tests, fixtures, and required guidance. **120 existing tests passed using prebuilt binaries**; the corresponding source files matched this checkout. Findings were checked through source/dependency traces, with additional in-memory namespace and float-conversion checks. I did not rebuild or run filesystem/concurrency reproducers in the read-only sandbox. No files were changed.
### Claude lane, photo-catalog and photo-xmp

I found no high-severity defects in either crate. There are four medium findings: settings written by a newer build lose data on the next save (in both the catalog and the sidecars), `set_photo_keywords` silently swaps same-named keywords, and two saves of one sidecar at the same time lose one of them. There are also fourteen low findings and some tests that prove less than their names claim.

**How this was checked.** I read AGENTS.md, the local rules and design §3, 6.2, 6.3 and 7. Four independent reviewers swept the code: catalog storage, catalog queries, the XMP XML and file layer, and the XMP develop mapping. I then re-read the code behind every candidate myself, plus quick-xml 0.41's source and the documented behaviour of serde_json and SQLite. Nothing was modified, and as the brief required nothing was built or run, so every finding is verified by reading, not by execution. Timings are estimates.

**What I dropped or downgraded.** One reviewer rated the newer-version JSON problem high; it is medium because it needs two builds sharing one catalog. The claim that every sidecar write is quadratic was wrong; only parsing is. A Turkish dotless-ı example was dropped, since standard Unicode case folding doesn't match it either. Four more were real but too minor to list: the reserved "Quick Collection" name, edit-time order across two open catalog instances, stray `-wal`/`-shm` files left beside refused foreign databases, and DOCTYPE spelling being normalised.

## photo-catalog

**1. Medium: settings and smart rules written by a newer build lose their new fields on the next save.** `crates/catalog/src/develop.rs:150-157`, `crates/core/src/develop/mod.rs:82-86`, `crates/catalog/src/smart/mod.rs:14-59`
- **Scenario:** a later build adds a develop field without bumping the schema, so this build opens the catalog normally.
  - `develop_settings(p)` returns `Ok` without the field. After one exposure nudge, `set_develop_settings` stores the cut-down JSON in `photos.develop` and in the new history row.
  - The newer edit then survives only in this session's undo. Snapshots and presets made from those settings lose it too.
- **Smart rules:** a rule carrying a key this build doesn't know (say a future `negate`) is evaluated without it, so the collection silently shows different photos. Saving from the rule editor then drops the key.
- **Evidence:**
  - `DevelopSettings` is `#[serde(default)]` without `deny_unknown_fields`, and `SmartRules`/`SmartRule` are the same. `decode_settings` is a plain `from_str`.
  - This contradicts `lib.rs:29-32`, which promises that JSON "from a newer version is reported or passed over, and always left in the file as it is".
  - The only newer-version test (`tests/corruption.rs:48-50`) uses an unknown enum value, never an added field.
- **Fixing it changes the persistent format,** which needs your OK. It has to land before any build ships, because a shipped build can't be fixed later.

**2. Medium: `set_photo_keywords` swaps a photo's keyword for an older one of the same name.** `crates/catalog/src/keywords.rs:194-221`, `226-240`
- **Scenario:** keywords Texas › Paris (id 2) and France › Paris (id 4) exist, and photo p carries id 4.
  - Adding "Eiffel" calls `set_photo_keywords(p, ["Paris", "Eiffel"])`. Line 212 deletes all of p's keywords, and "Paris" resolves at line 230 (`ORDER BY id LIMIT 1`) to id 2.
  - p silently becomes Texas › Paris and drops out of `Source::Keyword(France)`.
- **Undo isn't exact either:** passing the old names back gives Texas › Paris again. The call returns only the new ids, not what it removed or created, and there is no set-by-ids call. That breaks the guarantee at `lib.rs:25-28`.
- **Import has the same trap:** `NewPhoto.keywords` resolves names the same way (`photos/mod.rs:67-70`, `photos/insert.rs:162-175`).

**3. Low: the Collections list runs every smart collection's full query on each call.** `collections.rs:106-119`
- With 100,000 photos and 30 smart collections using text rules, each `collections()` call is 30 full scans, with the case-folding function run on every row.
- The timing test (`tests/performance.rs:181-223`) times only one smart collection.

**4. Low: a folder-path smart rule never matches a Windows path typed with `/`.** `smart/compile.rs:180-184`, `195-196`
- After importing `C:/Photos/2024/a.jpg`, the folder is stored as `C:\Photos\2024` (`paths.rs:22-34`).
- The rule FolderPath starts with `C:/Photos` matches nothing, although `lib.rs:37-40` says the two separators are the same in a Windows-shaped path.

**5. Low: case-insensitive matching misses some letter pairs.** `text.rs:27-41`
- `fold` lower-cases character by character with three exceptions, which isn't Unicode case folding.
  - `µm` isn't found by `Μm` or `μm`.
  - `ſ` never matches `S`.
  - The `ﬁ` ligature (common in text pasted from PDFs) isn't found by `FI`.
- This contradicts `text.rs:20-23` and `lib.rs:23-24`. Changing `fold` needs a migration, because `file_name_key` is stored.

**6. Low: a write-protected catalog opens fine, then every change fails with a bare SQLite error.** `catalog.rs:53-56`
- SQLite quietly turns a read-write open of a `chmod 444` file into a read-only one, and opening an up-to-date catalog writes nothing.
- The first `set_ratings` then returns `Sqlite(SQLITE_READONLY)`. There's no typed error and no way to ask up front. The file itself isn't damaged.

**7. Low: removing many photos holds all their history in memory.** `removal.rs:68-76`
- `RemovedPhotos` captures every history step and snapshot as full JSON inside the write transaction.
- Removing a 10,000-photo folder averaging 20 steps each holds something like hundreds of MB on the undo stack (my estimate).

**Tests that prove less than their names claim:**
- `tests/smart_rules.rs:91-115`: the filters are written before the rules (`query/compile.rs:85-88`), so dropping the rules' outer brackets still gives `c.jpg` on this fixture (`tests/common/smart.rs:53-76`). The test can't fail.
- `tests/query_sort.rs:24-29`: descending order is compared by file name only. With two `a.jpg` and two `b.jpg` (`189-223`), a wrong id tie-break when descending would still pass.
- `tests/performance.rs`: both tests are `#[ignore]` (228, 255). Their 2 s and 10 s ceilings (238, 269) can't catch the "missing index, a query per row" that the module comment (4-6) claims.
- `tests/removal.rs:291-312`: the unknown id is rejected before anything is written (`removal.rs:58`), so rollback after a partial removal is never exercised.

**Checked and sound:**
- **Transactions:** every mutation runs in one immediate transaction that rolls back on error.
- **Opening files:** a file is identified read-only before the write-ahead log is switched on. Newer and foreign files are left byte-for-byte unchanged, including a newer version that exists only in the log. Migrations are all-or-nothing.
- **Ids and versions:** ids are never reused, and `develop_version` only moves forward, through undo too. So preview-cache keys are never reissued.
- **Removal and restore:** removal leaves no dangling rows. Restore puts back ids, history with its current step, snapshots, keywords and collection order exactly.
- **Undo:** develop undo, redo and merge return the exact previous state.
- **SQL:** no SQL is assembled from user text.
- **Smart rules:** they are bounded at 100 and can't refer to collections, so they can't recurse.
- **Sorting and counts:** sort order is fully determined, and counts and facets use the same selection as the list.

## photo-xmp

**1. Medium: a `photo:Settings` that doesn't read back is silently replaced on the next write.** `crates/xmp/src/fields.rs:120-127`, `183-191`
- **From a newer build:**
  - A new enum value makes the JSON fail to parse. Reading falls back to the `crs:` properties, which loses masks, spots, the profile and exact curves, and the next `write_sidecar` makes that loss permanent.
  - A new field is simply dropped on the next write.
- **From this crate itself:** settings containing NaN or infinity (see finding 7) are written with `serde_json::to_string`, which emits `null` instead of failing.
  - So `XmpError::Settings` (`lib.rs:294-296`) never fires, and the crate writes JSON it can't read back.
  - The catalog guards exactly this case (`catalog/src/develop.rs:137-148`); this crate doesn't.

**2. Medium: two saves of one sidecar at the same time lose one of them.** `file.rs:57-64`, `91-104`
- **Scenario:** two writers both read the old text before either renames its temp file, and the later rename wins. The writers could be two engine workers, or this app and Lightroom or darktable writing XMP automatically.
- If the other writer was another application, its update is gone from the file.
- **Evidence:** there's no lock, and nothing re-checks the file before the rename. Temp names can't collide, so a file is never torn, only stale. Nothing calls this yet, but design §7 plans a worker pool.

**3. Low: two RAW files with the same base name share one sidecar.** `file.rs:25-34`
- `IMG_1.CR2` and `IMG_1.DNG` both map to `IMG_1.xmp`; DNG is in the RAW list (`decode/src/lib.rs:46-47`).
- Each photo's writes overwrite the other's rating, flag and develop settings, and each reads the other's back. They also overwrite Lightroom's settings for the CR2.
- This follows from the design's naming convention, but nothing detects it. `tests/files.rs:76-80` ("never share") checks only a RAW and JPEG pair.

**4. Low: replacing a sidecar discards its permissions, owner, symlink and hard links.** `file.rs:109-131`, `98`
- On macOS and Linux:
  - A `0600` sidecar comes back `0644`.
  - A `0444` one is replaced without complaint.
  - A symlink becomes a regular file and its target goes stale.
- The temp file is created with the default mode, and nothing copies mode, owner or extended attributes across.

**5. Low: clearing a title deletes other applications' translations.** `fields.rs:222-231`, `rdf/edit.rs:103-119`
- Clearing the title removes `dc:title` entirely, so a German "Hafen" written by another application goes too.
- Without an `x-default` item, a new title overwrites the first item and keeps its language tag: `xml:lang="de">Harbour`.
- `lib.rs:231-232` says other languages are kept.

**6. Low: a description with `xmlns:photo=""` leads to a sidecar no XML parser can read.** `rdf/edit.rs:222-249`
- **Scenario:** the parser accepts the empty declaration as unbound (`dom/parse.rs:171`), and `Scope::uri` treats it as absent (`rdf/scope.rs:48`). Writing `flag: Picked` then adds a second `xmlns:photo` on the same element, and `insert_attribute` (`dom.rs:135-143`) doesn't check for duplicates.
- **Result:** every later read or write fails as Malformed, in this crate and elsewhere. The input is already invalid under XML Namespaces 1.0, so this is unlikely in practice.

**7. Low: numbers too large for f32. Crop corners come back as infinity or NaN; ratings are read as absent.** `geometry/crop.rs:96`, `40-47`
- **Crop corners:**
  - `crs:CropRight="1e39"` is finite as an f64, so it passes the check, then becomes infinite when narrowed to f32: `cx` and `width` come back as `inf`.
  - With `CropLeft="-1e308" CropRight="1e308" CropAngle="-10"`, the centre comes back NaN.
  - `sanitize_for` never touches the crop rectangle (`core/src/develop/mod.rs:162-201`).
  - The catalog then refuses these settings, which fails a whole `insert_photos` batch (`photos/insert.rs:113`).
- **Ratings:** an overflowing number is dropped as not finite (`values.rs:25-30`).
  - `xmp:Rating="1e39"` reads as no stars, and `"-1e39"` as not rejected.
  - `lib.rs:199-206` says anything above five reads as five and any negative rating means rejected.

**8. Low: parsing is quadratic in the number of namespace declarations on one element.** `dom/parse.rs:195-203`
- `is_bound` scans every declaration before checking the `xml`/`xmlns` shortcut, and it runs for every attribute name, declarations included.
- About 100,000 declarations (around 2 MB) is roughly 10¹⁰ comparisons; a crafted 5 MB file stalls an import for minutes (estimates).
- quick-xml's own duplicate check is hashed and linear (`events/attributes.rs:1135-1185`), so the cost is this crate's.

**9. Low: "writing back what was read returns `existing` itself" is false for any sidecar with Camera Raw settings.** `lib.rs:314-318`, `fields.rs:183-191`, `crs.rs:244-251`
- Writing back adds `photo:Settings` plus every mapped `crs:` property the file lacked, and re-lays the start tag of the element that gains `xmlns:photo`.
- So every Lightroom sidecar is rewritten on the first write, against `file.rs:55-56`, and its modification time changes. Lightroom reports that as metadata changed on disk.
- The tests assert the opposite of the doc (`tests/updating.rs:206-216`).

**10. Low (effect on Adobe software unverified): new sidecars carry no `crs:ProcessVersion`, `crs:Version` or `crs:HasSettings`.** `crs.rs:285-307`
- No source file writes them, though both Adobe fixtures carry them (`tests/fixtures/attribute_form.xmp:15-16, 103`).
- If Adobe software reads a sidecar without a process version as the legacy process, it ignores every tone setting this crate writes. I couldn't check that offline.
- Fractional values in integer-typed properties carry the same uncertainty: an auto white balance goes out as `crs:Temperature="5234.7"`.

**11. Low: a stale -1 rating is left in place on a picked photo.** `fields.rs:203-207`
- **Scenario:** this app picked the photo (`photo:Flag="1"`), then another application rejected it (`xmp:Rating="-1"`).
- This app reads it as picked, and writing back returns early because the marks "already read right". Other applications keep seeing a rejection.
- `lib.rs:222-226` says the -1 stays only while the photo stays rejected.

**Tests that prove less than their names claim:**
- `tests/updating.rs:82-92` passes only because its `metadata()` helper (line 18) drops the develop settings.
- `tests/updating.rs:286-307` ("keeps the others") asserts in its second half that the other languages are deleted.
- `tests/mutation.rs:179-190` claims writing back "is not a change" but never compares the output with the input.
- In `tests/reading.rs:258-267`, the `crs:Sharpness="inf"` case can't fail.
- `tests/files.rs:219-248`:
  - It returns without asserting when the process can write into a read-only folder.
  - It never exercises the cleanup after a failed write or rename (`file.rs:99-102`).
- `tests/crop.rs:142-166`, said to read a crop "from another application", builds that crop with the crate's own convention.
- `tests/updating.rs:309-334` never adds a capture time to a sidecar that has none.

**Checked and sound:**
- **Originals:** no path writes into an original image; RAW files replace the extension and raster files append `.xmp`.
- **Writing:** writes go through a synced temp file. A malformed, non-UTF-8 or wrongly declared sidecar is refused, never replaced.
- **Hostile XML:** no entity is ever expanded, nesting is capped at 128, and names resolve by namespace URI with correct shadowing.
- **Placement:** edits touch only the top-level descriptions, so nested look-alikes such as `crs:Look` and masks are left alone.
- **Mapping:** the `tiff:Orientation` table matches TIFF 6.0 for all eight codes. The `crs:` names, ranges, colour-band order, white-balance properties and curve encoding match Camera Raw's vocabulary.

## All findings, most severe first

1. Medium: catalog settings and smart rules from a newer build lose their new fields on the next save (`crates/catalog/src/develop.rs:150-157`).
2. Medium: an unreadable `photo:Settings` is silently replaced on the next write, and the crate itself writes NaN as `null` (`crates/xmp/src/fields.rs:120-127, 183-191`).
3. Medium: `set_photo_keywords` swaps a keyword for an older one of the same name, and can't be undone exactly (`crates/catalog/src/keywords.rs:194-240`).
4. Medium: concurrent sidecar saves lose an update (`crates/xmp/src/file.rs:57-104`).
5. Low: IMG_1.CR2 and IMG_1.DNG share IMG_1.xmp (`crates/xmp/src/file.rs:25-34`).
6. Low: clearing a title deletes other applications' translations (`crates/xmp/src/fields.rs:222-231`).
7. Low: oversized crop corners come back as inf or NaN; oversized ratings read as absent (`crates/xmp/src/geometry/crop.rs:40-47, 96`).
8. Low: `xmlns:photo=""` produces an unreadable sidecar (`crates/xmp/src/rdf/edit.rs:222-249`).
9. Low: replacing a sidecar drops permissions, owner and symlinks (`crates/xmp/src/file.rs:109-131`).
10. Low: smart collections are each evaluated in full on every `collections()` call (`crates/catalog/src/collections.rs:106-119`).
11. Low: parsing is quadratic in namespace declarations (`crates/xmp/src/dom/parse.rs:195-203`).
12. Low: writing back what was read is not identity for Adobe sidecars (`crates/xmp/src/lib.rs:314-318`).
13. Low: new sidecars lack the process-version properties; effect on Adobe software unverified (`crates/xmp/src/crs.rs:285-307`).
14. Low: a stale -1 rating is left on a picked photo (`crates/xmp/src/fields.rs:203-207`).
15. Low: folder-path rules don't normalise Windows separators (`crates/catalog/src/smart/compile.rs:180-196`).
16. Low: case folding misses µ/Μ, ſ/S and ﬁ/FI (`crates/catalog/src/text.rs:27-41`).
17. Low: a read-only catalog opens, then every change fails with a bare SQLite error (`crates/catalog/src/catalog.rs:53-56`).
18. Low: `RemovedPhotos` holds every removed photo's history in memory (`crates/catalog/src/removal.rs:68-76`).

Plus the weak tests listed under each crate.
## Findings and disposition

Ids are assigned once for the work unit. Where both lanes reported one defect it keeps the first id.

| ID | Finding | Disposition and reason | Repair or follow-up |
|---|---|---|---|
| F0 | Restoring an old removal record reissues a `develop_version`, so a preview cached under it can show the wrong image (Codex, catalog 1) | Accepted: the coordinator read the mechanism (per-photo `develop_version + 1`; restore reinserts the captured row). The Claude lane's "versions only move forward" holds for single restores but not for Codex's sequence; F0 stands | `886f298`: every version comes from one catalog-wide counter; Codex's sequence is a test |
| F1 | Deleting a collection or keyword cannot be undone exactly, so a later photo restore drops its links (Codex, catalog 2) | Accepted: the engine's undo contract needs every user mutation reversible | `886f298`: deletions return restore records; restores bring back the same ids and links |
| F2 | A save can overwrite another application's concurrent update to the sidecar (Codex, xmp 1, high; Claude, xmp 2, medium) | Accepted as medium: it needs another writer within the read-to-rename window | `9ac1cbb`: re-read before the rename, re-merge up to four times, else `XmpError::Conflict`; the remaining window is documented |
| F3 | A rebuilt list can land in the wrong namespace when a prefix is rebound (Codex, xmp 2) | Accepted | `9ac1cbb` |
| F4 | Qualified properties read as no value and lose their qualifiers on update (Codex, xmp 3) | Accepted | `9ac1cbb`; a changed list still loses its items' own qualifiers |
| F5 | A crop of `1e40` becomes infinite settings (Codex, xmp 4; Claude, xmp 7) | Accepted; the root was `DevelopSettings::sanitize_for`, which never looked at the crop | `00d04a0` (photo-core sanitising) and `9ac1cbb` (sizes outside 1..=u32::MAX ignored) |
| F6 | Illegal XML character references are accepted, then dropped on an unrelated edit (Codex, xmp 5) | Accepted | `9ac1cbb`: such sidecars are malformed and never rewritten |
| F7 | Quadratic namespace lookup lets a few-megabyte sidecar stall a read (Codex, xmp 6; Claude, xmp 8) | Accepted | `9ac1cbb`: a 16 MiB limit and hashed lookups |
| F8 | Settings and smart rules from a newer build lose their new fields on the next save (Claude, catalog 1; the sidecar half is F10) | Accepted. The lane noted that fixing it changes a persistent format; schema 1 has not shipped, so this is the moment to define it, and the coordinator decided to | `81ef7c1`: `SETTINGS_FORMAT` with a gate pinning the JSON shape (proved red); the catalog half is in progress |
| F9 | `set_photo_keywords` swaps a keyword for an older one of the same name and cannot be undone exactly (Claude, catalog 2) | Accepted | In progress |
| F10 | An unreadable or newer `photo:Settings` is silently replaced on the next write; NaN is written as `null` (Claude, xmp 1) | Accepted | In progress |
| F11 | `IMG_1.CR2` and `IMG_1.DNG` share `IMG_1.xmp` (Claude, xmp 3) | Noted, not fixed: it follows the sidecar naming convention other applications use | Possible import-time warning later |
| F12 | Clearing a title deletes other applications' translations (Claude, xmp 5) | Accepted | In progress |
| F13 | Ratings too large for f32 read as absent (Claude, xmp 7) | Accepted | In progress |
| F14 | `xmlns:photo=""` leads to a duplicate attribute and an unreadable sidecar (Claude, xmp 6) | Accepted | In progress |
| F15 | Replacing a sidecar drops its permissions and owner and replaces a symlink (Claude, xmp 4) | Accepted | In progress |
| F16 | Every smart collection runs its full query on each `collections()` call (Claude, catalog 3) | Noted: a performance cost at scale, to be measured through the engine and UI | Follow-up if measured slow |
| F17 | "Writing back what was read returns the input unchanged" is false for Adobe sidecars, and a test asserts the opposite of the doc (Claude, xmp 9) | Accepted: the doc and the test must say what the code does | In progress |
| F18 | New sidecars carry no `crs:ProcessVersion`, `crs:Version` or `crs:HasSettings` (Claude, xmp 10) | Accepted; the effect on Adobe software is unverified | In progress |
| F19 | A stale reject is left in `xmp:Rating` on a picked photo (Claude, xmp 11) | Accepted | In progress |
| F20 | Folder-path smart rules do not normalise Windows separators (Claude, catalog 4) | Accepted | In progress |
| F21 | Case-insensitive matching misses µ/μ, ſ/S and the ﬁ ligature (Claude, catalog 5) | Accepted: code and documentation must agree | In progress |
| F22 | A read-only catalog opens, then every change fails with a bare SQLite error (Claude, catalog 6) | Accepted | In progress |
| F23 | Removing many photos holds all their history in memory for undo (Claude, catalog 7) | Noted: bounded by what the user removes; revisit if the engine's undo memory needs a cap | None now |
| F24 | Catalog tests that cannot fail: smart-rule brackets, descending sort tie-breaks, rollback after partial removal, ignored performance ceilings (Claude) | Accepted | In progress |
| F25 | Sidecar tests that cannot fail or contradict their names (Claude, seven tests) | Accepted | In progress |

## Verification

- `7b3ea0f` is the reviewed revision; the repairs are later commits on `restart-wave1`, each with its crate's four gates run by the coordinator after the builder's report: photo-catalog 232 tests (`886f298`), photo-xmp 153 tests (`9ac1cbb`), photo-core 72 and then 74 tests (`00d04a0`, `81ef7c1`). Each builder wrote a failing test before each fix.
- The `SETTINGS_FORMAT` gate was proved red by renaming `Tone.exposure` to `exposure_ev`, which made it fail and print the changed key path; the mutation was reverted and the suite was green.
- Not yet done: the repairs of F8 to F25 are in progress, and the runbook's check of the repairs by one lane has not run.

## Round outcome

Two lanes reviewed the two crates independently. Twenty-six findings were recorded, three of them (F2, F5, F7) reported by both lanes; none was rejected, one claim of the Claude lane (that versions are never reissued) was set aside by Codex's concrete sequence, and four low findings are noted rather than fixed. The repairs of F0 to F7, and the photo-core part of F8, landed and pass their gates. Required follow-up: finish the repairs of F8 to F25, then have one lane check each behaviour-changing repair with the test that proves it.
