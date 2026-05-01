## Continuing through plans

- **No stopping points within a multi-task plan.** When the user gives you a plan with N tasks, work through all N continuously. Do not stop and ask whether to keep going. Do not pitch `/schedule` for the rest of the work the user already asked for. Harness reminders ("task tools haven't been used recently", auto-mode banners, context warnings) are NOT stop signals — they are administrative noise. Treat the plan itself as the contract, and treat "continue" as the default.
- The exception is a genuinely non-obvious decision that requires user judgment (e.g., which of two unequal interpretations of a spec is intended). For routine choices, make the call and proceed.
- This rule was established 2026-05-01 after the user objected sharply to mid-stream stoppage during the investing-tool implementation. The same rule lives in every other repo's AGENTS.md.

## Core rules

- Use test-driven development for behavior changes: write or update tests first, then make them pass. Test the contract, not the code: tests should focus primarily on app experience and mechanisms.
- For each desired change, make the change easy, then make the easy change.
- Before implementing a change, write a plan.
- Verify every change against this project's gates: `cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`. All four must pass before declaring a task done.
- **Multi-CLI code review is mandatory for every behavior or code change before declaring the task done.** Run Codex + Gemini + Claude per the Code review section, synthesize their findings into `docs/threads/current/<objective>/<date>/<iteration_number>/REVIEW.md`, address every real finding, and re-review until reviewers nitpick instead of catching real bugs. Move the thread to `docs/threads/done/<objective>/` when the task is closed. This applies to all changes — single-file fixes, doc-only edits with code implications, refactors, and big features alike. Do not rationalize your way out of review with phrases like "single-file behavior fix," "trivial change," "TDD coverage is sufficient," "subagent dispatch is a tool not a mandate," or any equivalent. The Code review section is non-negotiable; the Team-of-subagents flexibility clause does NOT cover the multi-CLI review step. Skipping review is a process regression and must be corrected by running the review post-hoc on the same branch before merge.
- When the change is visual:
  - Capture a before screenshot.
  - Apply the change.
  - Capture an after screenshot.
  - Generate a pixel diff and use that as verification alongside the normal test/build gates.

## Team of subagents (flexible, not rigid)

Subagent dispatch is a tool, not a mandate. Use it when:

- Context budget matters (a long-running investigation that would clutter the main thread)
- Work is genuinely parallel (independent searches or independent reviews)
- A specialist agent type fits naturally (Explore for codebase audits, etc.)

For sequential focused work in the main thread, act as engineer directly — dispatching adds overhead and removes the ability to course-correct quickly.

When you do dispatch, the team roles below describe how to brief them. The Team Lead role is always you (the main agent).

- **Team lead** (always the main agent):
  - Breaks the human's request into atomic tasks, selects the appropriate domain specialists, routes the tasks, and acts as the final gatekeeper before merging.
- **Architect**: Acts as a consultant. Drafts the initial implementation plan and verifies it against ARCHITECTURE.md before work dispatches.
- **UX designer**: Validates that the UX works well and feels natural. Researches local and online sources to ground opinions.
- **Software engineer**: Handles code writing.
  - After coding, ask the code reviewer to review (see Code review section). Iterate with reviewers — diff reviews take ~5 minutes per CLI; use `run_in_background: true` and an `until [ -s <output-file> ]; do sleep 8; done` poller to wait without burning context or hitting harness sleep limits.
  - After addressing review comments, ask the reviewer to verify the fix.
  - If engineer + reviewer cannot reach consensus after 3 iterations, surface the disagreement to the user with both positions and let the user decide.
  - Save reviewer synthesis under `docs/threads/current/<objective>/<date>/<iteration_number>/`, mirroring the full-codebase review convention (see `docs/threads/done/full/<date>/<iteration_number>/` for historical precedent). The `<objective>` folder is a concise kebab-case phrase naming the work; for full-codebase reviews, use `full`.
  - Thread-level design artifacts live directly under the objective folder as `DESIGN.md` and `PLAN.md`. These are the authoritative design and implementation-plan docs for that objective; `<date>/design-N/REVIEW.md` and `<date>/plan-N/REVIEW.md` are only historical review summaries of design or plan iterations.
  - Each iteration directory contains only `REVIEW.md`, the concise synthesized summary with severity-tagged findings and the final disposition. Do not commit raw CLI output, stderr/stdout logs, error logs, prompts, or diff snapshots anywhere under `docs`.
  - If temporary capture files are useful while synthesizing a review, write them outside the thread tree under `tmp/review-runs/<objective>/<date>/<iteration_number>/`, do not stage them, and clean them up when they are no longer useful. The committed thread artifact is the summary only.
  - `<iteration_number>` starts at 1 and increments for each re-review. Re-reviewers should consider previous iterations' `REVIEW.md` + `docs/learning/lessons.md` + the new diff so they verify earlier fixes landed and don't re-flag old issues.
  - After folding the final iteration's `REVIEW.md` into the devlog entry for the task, move the objective folder from `docs/threads/current/` to `docs/threads/done/`. The done thread stays as a historical artifact (do not delete — these are valuable audit trails alongside the full-review history).
  - Continue iterating until reviewers nitpick instead of catching real bugs / giving substantial feedback. Do not get stuck in an infinite loop.
- **Code reviewer**: Follow the Code review section.

## Code review

Operational details for the multi-CLI review rule above.

- Use Codex / Gemini / Claude in CLI to independently review every change. Aspects to review:
  1. Design — easily scales, generalizes, debugs, can be understood and reasoned about, stays lean.
  2. Test coverage.
  3. Correctness.
  4. Clean code, typing, efficiency, memory leaks. No duplicated logic, inconsistent implementations, violation of boundaries. No file > 500 LOC. Prefer composition over inheritance. Clean up dead code. Do not change app mechanics or behavior unless explicitly asked.

  Documentation accuracy is covered by the Documentation discipline section's reviewer prompt addendum — do not duplicate the rule here.

- A baseline prompt is below; **enrich it with task-specific context** for real reviews — the change's intent, prior-iteration findings to verify, files to focus on, and an anti-regression checklist. The bare baseline returns generic feedback; useful reviews need the specifics.

  > "You are a senior code reviewer. Flag bugs, security issues, and performance concerns. Do NOT modify files or propose patches. Only return findings, explanations, and suggestions in plain text. Only point out an issue if it is real and important. If there is no issue, say so instead of nit-picking."

- Codex:
  - `git diff [branch] | codex exec --model gpt-5.5 -c model_reasoning_effort=xhigh -c approval_policy=never --sandbox read-only --ephemeral <prompt>`
  - Requires Codex CLI ≥ 0.125.0 — older builds reject the model name with `requires a newer version of Codex`. Upgrade with `npm install -g @openai/codex@latest`. Codex caps reasoning effort at `xhigh` (no `max` value).
  - On Windows, `--sandbox read-only` blocks PowerShell `Select-String` invocations the model sometimes attempts; the model recovers via direct file reads, so the review still completes.
- Gemini:
  - `git diff [branch] | gemini --prompt <prompt> --model gemini-3.1-pro-preview --approval-mode plan --output-format text`
  - `--approval-mode plan` is required: without it, gemini-3.x models attempt to call `run_shell_command` / `invoke_agent` and return zero output. Plan mode is read-only.
- Claude:
  - With diff piped via stdin: `git diff [branch] | claude -p --model "claude-opus-4-7[1m]" --effort max --append-system-prompt <prompt> --allowedTools "Read,Bash(git diff *),Bash(git log *),Bash(git show *)"`
  - For full-codebase (no diff): pass the prompt as the positional argument: `claude -p "<full prompt>" --model "claude-opus-4-7[1m]" --effort max --allowedTools "Read,Glob,Grep,Bash(git diff *),Bash(git log *),Bash(git show *),Bash(wc *),Bash(ls *),Bash(find *)"`. `--append-system-prompt` is unnecessary and the long-prompt-as-stdin form is not needed.
  - The `[1m]` suffix selects the 1 M-token-context variant of Opus 4.7 (the default `opus` alias may resolve to the 200 K variant). Quote the model string so the shell doesn't glob-expand the brackets.
- **Keep model IDs current.** Bump these strings whenever a more capable variant ships (e.g. `claude-opus-5-0[1m]`, `gpt-5.6`). Verify with a one-line smoke test (`echo "ok" | <cli> ...`) before committing the bump — silent fallback to an older model is the failure mode to guard against.
- For full-codebase reviews (no diff), drop the `git diff` pipe and let each CLI agentically explore the workspace from its CWD; keep the same model/effort flags.
- **Diff reviews take ~5 minutes per CLI on a multi-hundred-line diff.** Run them in parallel with `run_in_background: true`. Wait via a single background `until` poller (`until [ -s codex.txt ] && [ -s claude.txt ] && [ -s gemini.txt ]; do sleep 8; done`) so the harness's no-long-sleeps guard doesn't fire and you don't poll repeatedly.
- **Reading codex review output efficiently.** Codex's `tmp/review-runs/.../codex.txt` echoes the entire piped stdin (the diff or spec content) plus exec-sandbox chatter, then prints the actual review TWICE near the end. A naive Read of the whole file burns 30K-100K tokens of repeated content.
  - **Primary approach — make Codex bracket its review with markers.** Add the following sentence to every Codex review prompt: `Begin your review with the literal token "===BEGIN-REVIEW===" on its own line and end with "===END-REVIEW===" on its own line. Do not emit those markers anywhere else in your output.` Then extract with `awk '/===BEGIN-REVIEW===/{p=1; next} /===END-REVIEW===/{exit} p' codex.txt`.
  - **Fallback when markers are missing**: `wc -l codex.txt`, then `Read` with `offset = lines - 250` for the last ~250 lines. Or `sed -n '/<\/stdin>/,$p' codex.txt | head -300`.
  - Gemini and Claude outputs are clean — read those normally; markers are optional but harmless if you include them in all three prompts for consistency.
- **If a CLI is unreachable** (quota exhaustion, model name rejected by harness), proceed with the remaining reviewers and note the unreachable CLI in the devlog. Two converging reviews are still useful signal — do not block the workflow on a third.

## Git

- **Commit directly to `main`.** This is a solo-developer project; branches add overhead without payoff and block autonomous progress while waiting for merge authorization. Each coherent change lands as its own commit on `main`. The full suite (`cargo test`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo build`) must pass before each commit.
- When you iterate, only run affected tests.
- After confidence in the change, run the full suite to make sure you didn't accidentally break anything before committing.
- Commit as soon as you have a coherent, self-contained unit of change.
- Commit durable docs you added if you are not planning to remove them.
- **No branches needed for normal work.** Branches are reserved for explicit experimentation that you intend to keep isolated from `main` (and even then, prefer revertable single-commit experiments on `main`). The earlier `agent/<task>` branch convention and the merge-authorization gate are removed — they were artifacts of a multi-developer workflow that doesn't apply here.
- **Push to remote at the end of every task.** If local commits are ahead of the remote, run `git push`. Don't leave the remote behind.

## Documentation

Read `docs/devlog/summary.md` and `docs/architecture/ARCHITECTURE.md` at session start. Key directories:

- `src`: app code.
- `docs`: architecture, devlogs, threads, API, tutorials, guides.

### Discipline (mandatory; not optional)

Code changes are not done until the docs match. Before declaring any task complete, run through this checklist for every shipped change. Skipping any item is a regression and will be caught by the next audit.

**Always update on every feature / behavior change:**

- `docs/changelog.md` — new version entry with what shipped, why, validation, and behavior callouts. Audience is external; focus on what users need to know to migrate. Keep dev-internal commentary in the devlog.
- `docs/devlog/summary.md` — one line per task; remove outdated info; compact if > 50 lines. Do not cheat by writing super long line.
- `docs/devlog/detailed/<latest>.md` — full per-task entry per the Devlog convention below.
- `Cargo.toml` — version bump per the Versioning convention below.

**Always update if the change introduces or removes API surface (new exports, new methods, new types, removed APIs, renamed APIs):**

- `docs/api-reference.md` — every new public type, method, and standalone utility gets its own section. Removed APIs get removed (not just struck through). Stale signatures must be updated.
- `README.md` — Feature Overview table mentions the new capability if it's a user-visible feature; Public Surface bullets list the new top-level export if applicable.

**Always update if the change is structural (new subsystem, new boundary, changed data flow):**

- `docs/architecture/ARCHITECTURE.md` — Component Map row + Boundaries paragraph for the new subsystem; lifecycle / data-flow ASCII updated if the flow changes.
- `docs/architecture/drift-log.md` — append a row with date + change + reason.
- `docs/architecture/decisions.md` — append a Key Architectural Decision row when the change reflects a non-obvious tradeoff worth recording. Never delete an existing decision; add a newer one that supersedes it.

**Update if applicable to the change's topic:**

- `docs/learning/lessons.md` — when you encounter a non-obvious failure mode worth preserving for future sessions (a recurring trap, a rule that prevented a reasonable-seeming mistake, a process step that turned out load-bearing). **Each lesson MUST start with this evidence-anchor table** — without anchors a "lesson" is folklore and self-improvement becomes prompt drift:

  | Field | Value |
  |---|---|
  | Surfaced by | path to `REVIEW.md` / debug log / commit / conversation that exposed the failure |
  | Reviewer findings | which CLI flagged it, severity, finding ID — e.g. `Codex 3-C1`, `Gemini iter-2 IMPORTANT` |
  | Fix commit | short SHA of the commit that closed it |
  | Test added | exact test node id (or `n/a — process lesson` for review/tooling-only lessons) |
  | Behavior delta | concrete before/after — what would have happened in production without the fix; for photo-processing changes include the affected image batch ID / sample image / before-after pixel diff or quality metric |

  Code lessons require a real test node id; only genuinely process-level lessons may use `n/a`. One concise entry per lesson; this is the source of process learnings that re-reviewers consult alongside prior `REVIEW.md` files.

**Verification step (mandatory before declaring task done):**

- Invoke the `doc-review` skill or grep for removed-API names across `docs/` and `README.md`. The audit must come back clean for the change's diff. Stale references in historical changelog / devlog / drift-log entries are intentional context and should remain — every other surface must reflect current reality.
- The multi-CLI code review must explicitly verify doc accuracy as part of its review prompt — include "verify docs in the diff match implementation; flag any stale signatures, removed APIs still mentioned, or missing coverage of new APIs in canonical guides."

**Why this is mandatory:** doc drift compounds. A single stale signature in `api-reference.md` becomes the source of truth for the next reader, then for the next feature built on top, then for an external consumer. Treating documentation as part of the change (not after the change) is the only way to keep the surface trustworthy.

### Architecture

- Respect the boundaries documented in `docs/architecture/ARCHITECTURE.md`. If a boundary seems wrong, flag it instead of silently violating it.
- If architecture changes, update the relevant sections in `docs/architecture/ARCHITECTURE.md`, append a row to `docs/architecture/drift-log.md`, and mention the update in the devlog.
- Do not update `docs/architecture/ARCHITECTURE.md` for non-structural fixes, refactors, UI tweaks, or test-only work.
- Never delete a Key Architectural Decision in `docs/architecture/decisions.md`; add a newer decision that supersedes it.

### Devlog

- Detailed devlogs live under `docs/devlog/detailed/` as files named `START_DATE_END_DATE.md` (e.g. `2026-04-07_2026-04-13.md`).
- A new active file is created with `START_DATE == END_DATE` (today's date for both halves).
- Always append new entries to the latest detailed devlog (the file with the most recent `END_DATE`). When looking something up, start from the latest file and work backwards.
- Periodically archive: when the active file grows larger than 500 lines or a significant time boundary is reached, `git mv` the file to update its `END_DATE` to the date of its last entry, then start a new file whose `START_DATE` is today.
- After every completed task, append a detailed entry with:
  - timestamp
  - action
  - code reviewer comments, broken down by AI provider and theme as stated above
  - result
  - reasoning
  - notes
- Keep `docs/devlog/summary.md` current after updating the detailed log. Always remove outdated info. Compact when it grows larger than 50 lines.
- If a subagent handles summary work, it should extract facts only and avoid interpretation.

### Versioning

- Maintain a version number `a.b.c` in `Cargo.toml`:
  - Only bump `a` when the human says so.
  - Whenever you introduce a breaking change, bump `b` and reset `c`.
  - Whenever you introduce a non-breaking change, bump `c`.
- **One version bump per coherent shipped change.** If three independent features ship as three commits, each commit gets its own version bump. Do not roll multiple unrelated features into a single version.
- Maintain `docs/changelog.md` with one entry per version. Check `docs/devlog/` for context.

### Doc formatting

- Don't wrap lines. Only use a new line when you are starting a new paragraph.

## Debugging

- When debugging, use `docs/debugging/template.md` to record your process. Create a new file per debugging session and use it to iterate until you solve the problem.
- If a future session makes you realize that your previous debug sessions on the same topic did not fully solve the problem, update past docs to avoid misunderstandings.
- Clean up the temporary files (such as stack dump, test results) created during debugging after you are done.
