# Architecture drift log

Append a row whenever the structure in `ARCHITECTURE.md` changes. Rows before the 2026-11-19 restart are in git history before commit `94bd2cf`.

| Date | Change | Reason |
| --- | --- | --- |
| 2026-11-19 | Replaced the single-crate iced application with a twelve-package workspace (`photo`, `photo-core`, `photo-color`, `photo-imaging`, `photo-develop`, `photo-decode`, `photo-catalog`, `photo-xmp`, `photo-export`, `photo-engine`, `photo-ui`, `photo-testkit`). `photo-core` implemented; the rest are skeletons. | Restart as a catalog-based parametric raw developer; see decisions 1–10. |
| 2026-11-20 | The root package gained `src/repo.rs` (the only owner of storage locations and the harness sandbox override) and a library target so the coming `harnessctl` can share it. Highlights and Shadows moved from stage 6 into stage 5, in front of the base curve. `photo-ui` depends on `photo-engine` and no longer on anything below it. Stored develop settings and smart rules carry format numbers. | `AGENTS.md`'s single-owner invariant for storage; RAW highlight recovery (design 5.5); review round 0, F8. |
