# Lessons evidence

The war story, the measurements and the anchor behind each line in [lessons.md](lessons.md). Not session-start reading.

## A file's identity changes on FAT and exFAT under macOS

**Anchor:** 2026-11-25, commit `f4ed351` (the defect) and `c14fe42` (the fix), found by the photo-export builder running the placement tests on real FAT32 and exFAT images made with `hdiutil create -fs MS-DOS` and `-fs ExFAT` and pointed at with `TMPDIR`.

**Gate it waits for:** a CI step on the macOS runner that runs `cargo test -p photo-export` with `TMPDIR` on each kind of image. Until then the tests only show it when someone runs them that way by hand.

Measured on such volumes: neither hard links nor an exclusive rename exist (both `ENOTSUP`), so export takes a name by an exclusive create and a copy; and a file's device and inode change when an empty file gets its first bytes, when an empty file is renamed, and on exFAT when it is cut to nothing and rewritten. `f4ed351` noted the identity of the name it created while the file was empty and compared it with the one read after the copy, so it never recognised its own file: on a FAT32 image the unique-name policy wrote `x.jpg`, `x-2.jpg`, and on past `x-999.jpg` before it was stopped; skip reported a file it had just written as skipped; overwrite failed. The stand-in file systems in the tests could not show it, because they run on APFS. `c14fe42` keeps the file open and reads both identities at the moment of each comparison.
