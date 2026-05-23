# Known-Human to Human Consolidation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate `known_human` behavior under canonical `human` checkpoint handling while preserving external CLI/IDE compatibility, historical `h_` attestation readability, and existing attribution/stats behavior.

**Architecture:** Keep `CheckpointKind::Human` as the only internal human checkpoint kind. Treat `known_human` and `mock_known_human` as explicit compatibility aliases at the CLI/control boundary, normalize them to `human` before they reach core logic, and preserve historical `h_` marker parsing as a read-compatibility concern. This is a boundary cleanup and terminology consolidation, not a schema migration.

**Tech Stack:** Rust 2024, git-ai CLI checkpoint handler, daemon/control API parsing, working-log JSONL serialization, authorship note diff/read paths under `refs/notes/ai`, integration tests under `tests/integration/`, verification via `task build`, `task test`, `task lint`, and `task fmt`.

---

## Source Requirements

- `docs/superpowers/plans/2026-05-20-known-human-to-human-requirements.md`
- Existing implementation patterns in:
  - `docs/superpowers/plans/2026-05-12-ai-attribution-restoration.md`
  - `docs/superpowers/plans/2026-04-21-p1p2-git2-migration.md`

---

## File Structure

- Modify: `src/commands/git_ai_handlers.rs`
  - Add explicit alias handling for `known_human` and `mock_known_human`.
  - Keep pathspec behavior aligned with canonical human checkpoints.
  - Update help text so the old names are clearly labeled as compatibility aliases.

- Modify: `src/daemon.rs`
  - Normalize any external/control checkpoint kind string that reaches daemon parsing.
  - Ensure outgoing serialized kind remains canonical `human`.

- Modify: `src/daemon/control_api.rs`
  - Keep request types compatible with old external inputs.
  - Document that `kind` values may arrive as compatibility aliases but are normalized before execution.

- Modify: `src/authorship/working_log.rs`
  - Keep `CheckpointKind` limited to `Human`, `AiAgent`, and `AiTab`.
  - Preserve one-way compatibility parsing only if external or historical inputs can still reach the parser.

- Modify: `src/commands/diff.rs`
  - Preserve historical `h_` marker handling.
  - Update misleading comments that refer to a current `KnownHuman` checkpoint kind.

- Modify: `src/authorship/virtual_attribution.rs`
  - Keep existing legacy human sentinel behavior intact.
  - Clarify comments around `human` vs `h_` compatibility semantics.

- Modify: `src/authorship/attribution_tracker.rs`
  - Audit comments around human/known-human attribution behavior.
  - Preserve current dominance behavior unless a test demonstrates an intentional change is required.

- Modify: `src/authorship/authorship_log_serialization.rs`
  - Remove or rewrite comments that imply unsupported `metadata.humans` behavior.
  - Clarify that prompt metadata is for AI-backed attestations, not historical human markers.

- Modify: `tests/integration/formatting_non_substantial_ai_attribution.rs`
  - Reframe most tests around canonical `human` behavior.
  - Keep at least one explicit compatibility test for `mock_known_human` if the alias remains supported.
  - Add a focused regression for `known_human --hook-input stdin` if the existing integration coverage can exercise the IDE/editor payload path.

- Modify: `tests/integration/stats.rs`
  - Rename stale comments that still use `known_human` terminology.
  - Keep assertions focused on AI-vs-human stats semantics.

---

## Task 1: Make CLI aliases explicit at the command boundary

**Files:**
- Modify: `src/commands/git_ai_handlers.rs`

- [ ] **Step 1: Add explicit normalization for `known_human` and `mock_known_human`**

Update checkpoint argument parsing so both compatibility names resolve to canonical human behavior before any downstream checkpoint handling occurs. Do not rely on the default human path to accidentally pick them up.

- [ ] **Step 2: Preserve canonical pathspec behavior**

Verify the alias path uses the same pathspec handling as canonical human checkpoints, including the existing `--hook-input stdin` flow used by IDE/editor integrations.

- [ ] **Step 3: Rewrite help text**

Update CLI help so `known_human` and `mock_known_human` are clearly described as compatibility aliases, not separate checkpoint kinds or presets.

- [ ] **Step 4: Add or update alias-focused integration coverage**

Extend the existing checkpoint-related integration coverage so one test proves `known_human` remains accepted, one test proves `mock_known_human` remains usable for compatibility/testing without introducing a distinct persisted kind, and one test or assertion verifies that alias-driven checkpoints still serialize as canonical `kind = "human"` in the working log.

- [ ] **Step 5: Cover the `--hook-input stdin` compatibility path if the repo already has a hook-payload fixture**

If an existing integration helper or fixture can exercise the IDE/editor payload path, add a regression that passes a minimal valid `known_human --hook-input stdin` payload and verifies it normalizes to the same canonical human checkpoint behavior as the non-hook alias path.

---

## Task 2: Normalize parser boundaries without changing persisted output

**Files:**
- Modify: `src/daemon.rs`
- Modify: `src/daemon/control_api.rs`
- Modify: `src/authorship/working_log.rs`

- [ ] **Step 1: Review all string-to-kind entry points**

Locate every place where an external or control-plane string becomes a checkpoint kind. Confirm whether `known_human` can reach the parser and, if so, normalize it to `CheckpointKind::Human` there.

- [ ] **Step 2: Keep serialized output canonical**

Ensure the canonical serialized `kind` remains `"human"` for human checkpoints. Do not add a persisted `"known_human"` value.

- [ ] **Step 3: Preserve the internal enum shape**

Keep `CheckpointKind` limited to the existing variants. Do not add a `KnownHuman` variant, and do not broaden working-log serialization to a new human kind name.

- [ ] **Step 4: Add parser-level regression coverage if needed**

If a direct unit test already exists for checkpoint-kind parsing, extend it with a `known_human` input case that resolves to `CheckpointKind::Human`. If no such test exists, keep this as a boundary assertion in the integration tests added in Task 1.

---

## Task 3: Preserve historical `h_` compatibility and clean misleading terminology

**Files:**
- Modify: `src/commands/diff.rs`
- Modify: `src/authorship/virtual_attribution.rs`
- Modify: `src/authorship/attribution_tracker.rs`
- Modify: `src/authorship/authorship_log_serialization.rs`

- [ ] **Step 1: Keep historical `h_` read semantics intact**

Retain the existing handling that classifies historical `h_` markers as human attribution in diff/blame-style reads. This work should not delete legacy support or reinterpret old data as unknown.

- [ ] **Step 2: Audit comment language in attribution code**

Replace “KnownHuman checkpoint” phrasing with one of three explicit terms, depending on context: canonical human checkpoint, compatibility alias, or historical `h_` attestation marker.

- [ ] **Step 3: Leave behavior unchanged unless a test proves otherwise**

If `attribution_tracker.rs` currently treats some `h_`-derived cases as AI-like due to sentinel comparisons, do not silently change that behavior. Either keep it and document it, or add a regression test that proves a deliberate change is required.

- [ ] **Step 4: Fix serialization comments to match actual schema**

Ensure comments in `authorship_log_serialization.rs` describe the fields that actually exist today. Do not imply a `metadata.humans` schema field if the format does not currently contain one.

---

## Task 4: Clean up tests so core behavior and compatibility behavior are explicit

**Files:**
- Modify: `tests/integration/formatting_non_substantial_ai_attribution.rs`
- Modify: `tests/integration/stats.rs`

- [ ] **Step 1: Convert core behavior tests to canonical `human` where appropriate**

For tests that are really proving the normal human checkpoint path, prefer canonical `human` usage so the test name and implementation match the behavior being validated.

- [ ] **Step 2: Keep one focused alias compatibility test**

Retain at least one test that intentionally calls `mock_known_human` to prove the compatibility alias remains supported. This test should be clearly framed as compatibility coverage, not as proof of a distinct internal kind.

- [ ] **Step 3: Update stale terminology in stats assertions and comments**

Rename comments such as `known_human_accepted=0` to canonical human terminology. Keep the existing stats semantics focused on AI acceptance vs human additions.

- [ ] **Step 4: Re-run the impacted integration tests**

Run the targeted test filters that cover the changed behavior before expanding to the full suite.

- [ ] **Step 5: Verify working-log output stays canonical**

Add or update a test assertion that inspects the resulting working-log entry and confirms the serialized checkpoint kind is `"human"`, never `"known_human"`.

---

## Verification Plan

Run the following after the implementation is complete:

```bash
task test TEST_FILTER=formatting_non_substantial_ai_attribution
task test TEST_FILTER=stats
task test TEST_FILTER=known_human
task build
task test
task lint
task fmt
```

Expected result: all commands pass, and no generated working-log entry serializes `kind = "known_human"`.

---

## Risks and Non-Goals

### Risks

- External IDE integrations may still send `git-ai checkpoint known_human --hook-input stdin`; removing the alias would break them.
- Historical repositories may contain `h_` markers that still need to render as human in diff/blame paths.
- Misleading test names can obscure whether a test covers core human behavior or only alias compatibility.

### Non-Goals

- Do not introduce a new internal `KnownHuman` enum variant.
- Do not migrate historical git notes or working logs.
- Do not design a new authorship schema for human attestations in this change.
- Do not remove the `known_human` compatibility alias until external integrations have migrated.

---

## Acceptance Criteria

- [ ] No internal enum or persisted working-log kind named `known_human` exists.
- [ ] `known_human` and `mock_known_human` remain explicit CLI compatibility aliases for human behavior.
- [ ] Alias inputs serialize as canonical `human`, not `known_human`.
- [ ] Help text clearly labels the old names as compatibility aliases.
- [ ] Tests intentionally cover both canonical human behavior and compatibility alias behavior.
- [ ] Historical `h_` marker handling is preserved and documented as compatibility behavior.
- [ ] Comments no longer imply a current `KnownHuman` checkpoint kind.
- [ ] `task build`, relevant tests, full `task test`, `task lint`, and `task fmt` pass.

---

## Self-Review Checklist

Before implementation starts, confirm the plan is consistent with the requirements document:

1. Spec coverage: every requirement in `2026-05-20-known-human-to-human-requirements.md` maps to at least one task above.
2. Placeholder scan: there are no TBD/TODO gaps or vague “handle edge cases” instructions.
3. Type/terminology consistency: the plan only uses `human` for the canonical checkpoint kind and reserves `known_human` / `mock_known_human` for compatibility aliases.
4. Scope check: this is one focused consolidation plan, not a broader authorship schema migration.
