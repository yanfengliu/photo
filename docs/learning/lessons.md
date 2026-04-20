# Lessons

Keep this file short, current, and actionable.

## Active Lessons
- 2026-04-20 - Keep responsive thumbnail grids driven by the latest window width, and share the same geometry constants between breakpoint math and rendering so returning from another tab cannot reuse stale layout assumptions.
- 2026-04-20 - Keep crop preview, status dimensions, actual-size zoom, and save/export on the same pixel-snapped crop rectangle, and make those calculations follow the crop that is currently visible rather than hidden edit state.
- 2026-04-20 - Keep drag affordances and drag behavior backed by the same pannability rule, and measure small visible offsets in pixels so fit-to-window images stay still while off-center images can still be dragged back into place.
- 2026-04-19 - When orientation changes affect layout, feed the same rotated dimensions into preview fit, actual-size zoom, status text, and save/export paths so the UI and output stay in sync.
- 2026-04-19 - Keep canonical docs paths aligned with automation and repo instructions. When the layout changes, add compatibility stubs at the old paths until all references are updated.
- 2026-04-19 - Compact summaries work better than long logs for day-to-day maintenance. Preserve the long-form history in dated detailed files and keep the summary focused on current state.
- 2026-04-19 - Keep supported image extensions centralized so directory scans, navigation, and file dialogs stay aligned when a new format is added.
- 2026-04-19 - When docs move to new canonical paths, keep the active devlog updated with validation and reviewer status until the new paths are fully verified.
