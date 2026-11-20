# Architecture drift log

Append a row whenever the structure in `ARCHITECTURE.md` changes. Rows before the 2026-11-19 restart are in git history before commit `94bd2cf`.

| Date | Change | Reason |
| --- | --- | --- |
| 2026-11-19 | Replaced the single-crate iced application with a twelve-package workspace (`photo`, `photo-core`, `photo-color`, `photo-imaging`, `photo-develop`, `photo-decode`, `photo-catalog`, `photo-xmp`, `photo-export`, `photo-engine`, `photo-ui`, `photo-testkit`). `photo-core` implemented; the rest are skeletons. | Restart as a catalog-based parametric raw developer; see decisions 1–10. |
