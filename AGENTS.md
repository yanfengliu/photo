## Core rules

- Use test-driven development for behavior changes: write or update tests first, then make them pass. Test the contract, not the code: tests should focus exclusively on app experience and mechanisms.
- For each desired change, make the change easy, then make the easy change.
- Before implementing a change, write a plan.
- Use a subagent to implement the plan such that the tests pass. For example, if the tech stack uses node, it should make sure `npx vitest run`, `npx tsc --noEmit`, and `npx vite build` pass.
- When the change is visual:
  - Capture a before screenshot.
  - Apply the change.
  - Capture an after screenshot.
  - Generate a pixel diff and use that as verification alongside the normal test/build gates.

## Team of subagents

- For every task from the user, create a stateless, ephemeral team of subgents to work together on tasks, then turn down the agents when you are done to avoid context rot.
- **Team lead**:
  - Responsibility: Breaks the human's request into atomic tasks, selects the appropriate domain specialists, routes the tasks, and acts as the final gatekeeper before merging.
  - If tests (`npx vitest run`, `npx tsc --noEmit`, etc.) fail or review consensus is not reached after 3 iterations, the Team Lead must execute a hard abort. It will `git reset --hard` the branch, dump the error logs and the failed approach into `docs/learning/lessons.md`, and spin up a completely fresh Architect and Engineer to write a brand new plan that explicitly avoids the failed approach.
- **Architect**:
  - Responsibility: Act purely as a consultant rather than an active driver. The Lead queries the Architect to draft the initial implementation plan and verify it against ARCHITECTURE.md before dispatching work.
- **UX designer**:
  - Responsibility: Make sure the UX mechanism works well and is fun. Research local and online sources to ground your opinions.
- **Software engineer**:
  - Responsibility: Handle all the code writing.
  - Reach out to the team if you have questions or need a second opinion. 
  - Iterate with the code reviewer. Reviews might take a long time. Be patient.
  - After addressing review comments, ask the reviewer to verify that you have successfully done so.
  - If the Software Engineer and the Code Reviewer cannot reach consensus after 3 iterations, escalate to the Tie-Breaker agent.
  - Write down the reviewer feedback from previous round(s) under `code_review/` as temp files. The reviewer should consider this info + `docs/learning/lessons.md` + your diff. After you summarize reviewer feedback into devlog, delete the temp files.
  - Continue this iteration loop until the reviewers seem to start nit-picking instead of catching real bugs / giving substantial feedback. Do not get stuck in an infinite loop.
- **Code reviewer**: Follow the code review section for detailed rules.
- **Tie breaker**: Use the high-reasoning model. Its prompt dictates that it must definitively choose to either ACCEPT the current diff (overriding the reviewer) or REJECT it with a mandatory, prescriptive patch. The Tie-Breaker's decision is final.

## Code review

- Use all of Codex / Gemini / Claude in CLI to independently review every change on the following aspects:
  1. Design.
    - Can easily scale, generalize, debug, be understood and reasoned about, and stay lean.
  2. Test coverage.
  3. Correctness.
  4. Clean code, typing, efficiency, memory leaks.
    - No: god class, large files, duplicated logic, inconsistent implementations, violation of boundaries.
    - Prefer composition over inheritance.
    - Clean up dead code.
    - Do not change app mechanics or behavior unless explicitly asked.
  5. Documentation.
    - Dev logs should be updated and maintained.
    - References to code should be up to date.
    - No outdated comments.
    - Learnings from debugging and friction points should be documented in `docs/learning/lessons.md`. The file should be actively maintained to not become long, tedious, or outdated.
- `base_prompt` for the code review agent: "You are a senior code reviewer. Flag bugs, security issues, and performance concerns. Do NOT modify files or propose patches. Only return findings, explanations, and suggestions in plain text."
- Optionally, use the @ symbol within `base_prompt` to include directory context for the best reasoning results.
- Codex:
  - `git diff [branch] | codex exec --model gpt-5.4 --model-reasoning-effort xhigh --sandbox read-only --ask-for-approval never --ephemeral <base_prompt>`
- Gemini:
  - `git diff [branch] | gemini -p <base_prompt> --model gemini-3-pro --thinking high`.
- Claude:
  - `git diff [branch] | claude -p --append-system-prompt <base_prompt> --allowedTools "Read,Bash(git diff *),Bash(git log *),Bash(git show *)"`

## Git

- When you iterate, only run affected tests.
- In the end, after you are confident about your change, run the full suite of tests to make sure you didn't accidentally break anything.
- Create a short-lived branch for every task (e.g., `agent/fix-tick-start`). Run the test suite on the branch. Only after all tests and visual pixel-diffs pass, merge into main using a fast-forward merge, and delete the branch.
- Commit durable docs you added if you are not planning to remove them.
- Commit as soon as you have a coherent, self-contained unit of change.

## Project docs

- Read `docs/devlog/summary.md` and `docs/architecture/ARCHITECTURE.md` at session start.
- Key directories:
  - `src`: app code.
  - `docs`: architecture, devlogs, reviews.
  - `design`: app and mechanism notes.

## Architecture

- Respect the boundaries documented there. If a boundary seems wrong, flag it instead of silently violating it.
- If architecture changes, update the relevant sections in `docs/architecture/ARCHITECTURE.md`, append a row to `docs/architecture/drift-log.md`, and mention the update in the devlog.
- Do not update `docs/architecture/ARCHITECTURE.md` for non-structural fixes, refactors, UI tweaks, or test-only work.
- Never delete a Key Architectural Decision in `docs/architecture/decisions.md`; add a newer decision that supersedes it.

## Devlog

- Detailed devlogs live under `docs/devlog/detailed/` as append-only files named `YYYY-MM-DD_YYYY-MM-DD.md` (e.g. `2026-04-07_2026-04-13.md`).
- Always append new entries to the latest detailed devlog (the file with the most recent `END_DATE`). When looking something up, start from the latest file and work backwards.
- Periodically archive: when the active file grows larger than 500 lines or a significant time boundary is reached, close it (freeze its `END_DATE` in the filename) and start a new file whose `START_DATE` is the next entry's date. Check if the start and end dates of all previous devlogs are still accurate.
- After every completed task, append a detailed entry with:
  - timestamp
  - action
  - code reviewer comments, broken down by AI provider and theme as stated above
  - result
  - reasoning
  - notes
- Keep `docs/devlog/summary.md` current after updating the detailed log. Always remove outdated info. Compact when it grows larger than 50 lines.
- If a subagent handles summary work, it should extract facts only and avoid interpretation.

## Debugging

- When debugging, use `docs/debugging/template.md` to record your process. Create a new file per debugging session and use it to iterate until you solve the problem.
- If a future session makes you realize that your previous debug sessions on the same topic did not fully solve the problem, update past docs to avoid misunderstandings.
- Clean up the temporary files (such as stack dump, test results) created during debugging after you are done.