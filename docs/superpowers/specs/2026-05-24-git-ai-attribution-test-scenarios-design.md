# Git AI Attribution Test Scenarios Design

## Purpose

Create a test-scenario map for Git AI's line-level authorship attribution behavior before writing new tests. The map should make it clear which AI, human, and untracked edit flows are already covered, which ones are duplicated, and which gaps are highest risk.

The output of this design is a lifecycle-oriented scenario matrix. It is not an implementation plan and does not add tests by itself.

## Recommended Organization

Use a lifecycle matrix as the primary structure, with mechanism tags on each scenario.

Each scenario should record:

- Lifecycle phase.
- Actor sequence.
- Checkpoint flow.
- Expected committed line attribution.
- Verification surface.
- Existing coverage.
- Priority.
- Mechanism tags.

Lifecycle phases keep the map tied to user-visible behavior. Mechanism tags still identify which subsystem a scenario exercises, such as checkpoint classification, working-log persistence, post-commit note generation, blame rendering, stats, rewrite-log handling, or worktree routing.

## Scenario Phases

### 1. Before First Commit

Cover new-file flows where edits happen before the first commit. These scenarios are highest priority because there is no previous committed baseline and checkpoint order determines ownership.

Core cases:

- AI creates a new file, then commit: every generated line is AI.
- Human creates a new file, AI appends before commit: original lines stay human, appended lines are AI.
- AI creates a new file, known human appends before commit: AI block stays AI, appended block is known human.
- AI creates a new file, uncheckpointed human appends before commit: AI block stays AI, appended block is human or unattributed human according to the current commit-note contract.
- AI creates a new file, known human modifies an AI line before commit: modified line becomes human, untouched AI lines remain AI.
- AI creates a new file, legacy `human` checkpoint captures an intervening edit, AI edits after that: legacy captured zone is unattributed human, later AI zone is AI.
- Human inserts between AI lines before first commit: inserted line stays human when explicitly checkpointed as known human.

Current nearby coverage exists in `tests/integration/simple_additions.rs`, especially first-commit and evergreen-style AI-then-human scenarios. The matrix should identify which of these are already covered and whether they assert the exact desired known-human versus legacy-human distinction.

### 2. Existing Committed Files

Cover edits on files with committed attribution history. These scenarios validate ownership transfer and preservation after normal edits.

Core cases:

- AI inserts new lines into human file: inserted lines are AI, untouched human lines remain human unless included in an actual changed hunk by the current attribution algorithm.
- AI replaces human line: replacement line is AI.
- Human replaces AI line: replacement line is human.
- Human deletes AI line: no AI attestation is emitted for pure deletion commits.
- AI deletes human lines and inserts replacement block: replacement block is AI, surviving untouched lines retain prior ownership.
- Multiple AI sessions edit the same file before commit: committed surviving lines are attributed to AI, with prompt/session metadata preserved according to existing metadata rules.
- Human and AI alternate over multiple commits: each commit's final line ownership is asserted after that commit.

Current nearby coverage exists in `simple_additions.rs`, `stats.rs`, and the iterative tests in `formatting_non_substantial_ai_attribution.rs`.

### 3. Formatting, Reflow, and Non-Substantial Edits

Cover changes where the text meaning may be similar but line structure changes. These scenarios are important because users often run formatters or accept AI formatting-only edits.

Core cases:

- AI expands one human line into multiple formatted lines: all new formatted lines are AI.
- AI changes indentation or whitespace on a human line: touched line becomes AI unless the current non-substantial edit rules intentionally preserve previous ownership.
- Human adds only leading or trailing whitespace to committed AI line: line keeps AI attribution when covered by the current edge-whitespace rule.
- Human changes an internal token on AI line while also changing edge whitespace: token-changed line becomes human, whitespace-only neighboring lines can remain AI.
- AI rewrites a block containing byte-identical separator lines: byte-identical lines that Git does not report as changed stay with previous ownership.
- AI reformats a markdown table or config block: actually changed rows are AI, unchanged headers/separators remain prior ownership.
- AI edits around a large human section: untouched middle section remains human.

Current nearby coverage exists in `tests/integration/formatting_non_substantial_ai_attribution.rs`.

### 4. Index and Staging Behavior

Cover cases where the working tree has more attribution state than the commit actually includes.

Core cases:

- AI changes are staged, then later AI changes remain unstaged: commit note only covers staged lines.
- AI changes are staged, then human unstaged change touches an adjacent line: committed attribution reflects the staged content only.
- Human stages only some AI-generated lines: staged AI lines are attributed, unstaged lines are ignored by committed-line assertions.
- Multiple AI sessions exist in working log, but only one session's changes are staged: commit metadata and attestations cover only staged changes.
- Explicit path checkpoint captures one file while another file has uncheckpointed edits: attribution only changes for the scoped file.
- Partial staging with newline-at-EOF side effects: committed-line helper expectations document any current limitations.

Current nearby coverage exists in `simple_additions.rs` around partial staging. The matrix should call out existing edge cases where the helper comments explain newline side effects.

### 5. Git Lifecycle Operations

Cover operations that move, rewrite, or preserve commits and working logs.

Core cases:

- Amend preserves authorship for amended AI content and handles unstaged AI content correctly.
- Reset soft and mixed reconstruct working logs for unwound AI commits.
- Reset with pathspec preserves attribution for non-reset files.
- Stash pop and stash apply preserve AI attribution and prompt metadata.
- Rebase preserves authorship notes across simple, conflict, `--onto`, and abort flows.
- Cherry-pick copies or adapts authorship from the source commit.
- Merge and squash merge preserve attribution for AI-introduced lines without leaking unrelated notes.
- Checkout and switch keep working-log state consistent when files move between branches.
- Linked worktrees route checkpoints and blame to the correct worktree-local storage.

Current nearby coverage exists in `subdirs.rs`, `rebase.rs`, `reset.rs`, `stash_attribution.rs`, `cherry_pick.rs`, `merge_rebase.rs`, `squash_merge.rs`, and `worktrees.rs`.

### 6. Agent Hook Realism

Cover supported agent presets so realistic hook inputs map to the same lower-level checkpoint semantics.

Core cases:

- Pre-edit hook records a human or legacy-human checkpoint as intended by the preset.
- Post-edit hook records an AI checkpoint scoped to the edited files.
- Bash or command-run hooks capture generated file changes without attributing unrelated files.
- Multi-file agent operations produce per-file attribution and prompt metadata.
- Agent-specific transcript/model/session metadata appears in the authorship log when AI lines are committed.
- Ignored files and ignored prompts are excluded consistently.

Current nearby coverage exists in agent preset files such as `claude_code.rs`, `codex.rs`, `cursor.rs`, `opencode.rs`, `windsurf.rs`, `github_copilot.rs`, and related comprehensive preset tests.

## Test Design Rules

Use the high-level `TestFile` fluent helper when the scenario only needs a normal human-versus-AI content setup. Use explicit file writes and explicit checkpoints when checkpoint order, checkpoint kind, first-commit behavior, or attribution holes are the subject of the test.

For checkpoint-sensitive scenarios:

- Write content with `std::fs::write`.
- Call `git-ai checkpoint mock_ai <path>` for AI changes.
- Call `git-ai checkpoint mock_known_human <path>` for explicitly known human changes.
- Call `git-ai checkpoint human <path>` for legacy or untracked-human zones.
- Commit using the test repo helpers.
- Assert after every commit with `assert_lines_and_blame` or `assert_committed_lines`.

Do not collapse known-human and legacy-human behavior into a single expected result. They represent different attribution meanings and should be tested separately when the distinction matters.

## Prioritization

Priority 1: first-commit and same-commit mixed attribution. These are closest to the core product promise and easiest to regress when checkpoint logic changes.

Priority 2: formatting and non-substantial edit behavior. These cases are common in real AI workflows and have subtle ownership expectations.

Priority 3: partial staging and explicit path checkpoint behavior. These cases catch commit-boundary bugs and prevent notes from overclaiming unstaged work.

Priority 4: lifecycle operations. Much of this area already has coverage, so new work should focus on gaps found by the scenario matrix rather than broad duplication.

Priority 5: agent hook realism. Add agent-specific tests when lower-level semantics are missing from a preset or when a preset transforms hook input in a unique way.

## Success Criteria

The completed scenario map should let a maintainer answer these questions quickly:

- Which attribution behavior is already covered by tests?
- Which coverage is duplicated or too broad?
- Which missing scenarios are highest risk?
- Which file should each future test live in?
- Which helper style should each future test use?
- Which subsystem is exercised by each scenario?

The first implementation batch should be selected from missing Priority 1 scenarios, then expanded only after those cases are covered and verified.

## Out of Scope

This design does not implement tests, change attribution logic, alter helper APIs, or update snapshots. It also does not redefine product semantics for ambiguous existing behavior; the scenario matrix should document current expected behavior first, then call out any behavior that needs a separate product decision.
