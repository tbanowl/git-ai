# Known-Human to Human Consolidation Requirements

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Consolidate `known_human` behavior under the canonical `human` checkpoint model while preserving external CLI/IDE compatibility and historical authorship note readability.

**Architecture:** Keep `CheckpointKind::Human` as the only internal human checkpoint kind. Treat `known_human` and `mock_known_human` as CLI compatibility aliases that normalize to `human` at the boundary. Preserve historical `h_` attestation parsing as a read-compatibility layer unless a separate schema migration explicitly replaces attested-human semantics.

**Tech Stack:** Rust 2024, git-ai CLI checkpoint handler, daemon checkpoint parsing, working-log JSONL, authorship notes under `refs/notes/ai`, integration tests under `tests/integration/`, verification via `task test`, `task build`, `task lint`, `task fmt`.

---

## Problem Statement

`known_human` was introduced as an explicit human-edit checkpoint protocol for IDE/editor integrations. The current core model has already converged on `CheckpointKind::Human`; there is no internal `CheckpointKind::KnownHuman` enum or working-log `kind` value.

The remaining inconsistency is at the API and terminology boundary:

- CLI help still advertises `known_human` / `mock_known_human`.
- Some tests still call `mock_known_human` to exercise explicit human checkpoint behavior.
- Historical authorship-note logic still recognizes `h_` author markers as human attestations.
- Comments and docs still use “KnownHuman” language, which implies an internal kind that no longer exists.

The required change is therefore a **consolidation**, not a broad deletion.

---

## Design Requirements

### Requirement 1: Canonical internal checkpoint kind is `human`

All internal runtime paths must continue to use `CheckpointKind::Human` for human checkpoints.

**Must hold:**

- `CheckpointKind` must not gain a `KnownHuman` variant.
- Working-log serialized checkpoint `kind` must remain one of:
  - `"human"`
  - `"ai_agent"`
  - `"ai_tab"`
- Daemon/control API internal requests must normalize human-compatible inputs to `CheckpointKind::Human`.
- Agent preset pre-edit checkpoints must continue to produce `CheckpointKind::Human`.

**Must not do:**

- Do not introduce a new on-disk working-log kind named `"known_human"`.
- Do not rewrite existing working logs just to rename human checkpoint kinds.
- Do not make `known_human` a first-class internal enum value.

---

### Requirement 2: Keep `known_human` as an explicit CLI compatibility alias

The CLI should explicitly accept old external entry points and normalize them to `human`.

**Aliases:**

- `git-ai checkpoint known_human ...` → canonical human checkpoint behavior.
- `git-ai checkpoint mock_known_human ...` → test/debug alias for canonical human checkpoint behavior.

**Expected behavior:**

- Alias handling must be explicit in `src/commands/git_ai_handlers.rs`; it should not rely on falling through to the default human path.
- Pathspec handling for aliases must match canonical human checkpoint pathspec behavior.
- `--hook-input stdin` payload handling for `known_human` must match the existing IDE/editor protocol if still supported by external integrations.
- Help text must clarify that `known_human` is a deprecated/compatibility alias for `human`, not a separate checkpoint kind.

**Compatibility rationale:**

Existing IDE integrations and historical docs call:

```bash
git-ai checkpoint known_human --hook-input stdin
```

Removing this command shape would break those integrations. The boundary should accept it, normalize it, and avoid leaking the old name deeper into the system.

---

### Requirement 3: Preserve historical `h_` attestation read semantics

`h_` author markers are not checkpoint kinds. They are historical authorship-note / attribution markers for explicit human attestations.

**Must hold:**

- Existing notes that contain `h_` markers must remain readable.
- `git-ai diff` must continue to classify historical `h_` markers as human attribution unless a separate schema migration replaces this behavior.
- Post-commit prompt hydration must continue to avoid treating `h_` markers as AI prompt hashes.
- Stats/blame behavior must not regress for repositories with older notes.

**Must not do in this change:**

- Do not delete all `h_` handling simply because `known_human` is being consolidated.
- Do not collapse attested historical human markers into unknown/no-data unless the product explicitly accepts that data loss.
- Do not introduce `metadata.humans` unless this becomes a separate authorship schema migration.

---

### Requirement 4: Clean up misleading terminology

Source comments, test names, and help text should distinguish three concepts:

1. **Canonical human checkpoint**: internal `CheckpointKind::Human` / serialized `"human"`.
2. **Compatibility aliases**: CLI names `known_human` and `mock_known_human` accepted for old integrations/tests.
3. **Historical attested-human marker**: `h_` author markers in old authorship/attribution data.

**Terminology guidance:**

- Prefer “human checkpoint” for runtime checkpoint behavior.
- Prefer “known_human compatibility alias” only at CLI/API boundaries.
- Prefer “historical `h_` human attestation marker” for note/attribution compatibility logic.
- Avoid comments that imply there is a current `KnownHuman` checkpoint kind.

---

## File-Level Change Requirements

### `src/commands/git_ai_handlers.rs`

**Required changes:**

- Add explicit handling for `known_human` and `mock_known_human` in checkpoint argument parsing.
- Normalize both aliases to `CheckpointKind::Human`.
- Preserve existing pathspec behavior for canonical human checkpoints.
- Preserve `--hook-input stdin` behavior for known-human IDE payloads.
- Update help text to say these names are compatibility aliases.

**Acceptance criteria:**

- `git-ai checkpoint known_human <path>` produces the same attribution behavior as canonical human checkpoint behavior for that path.
- `git-ai checkpoint mock_known_human <path>` remains usable by tests but does not create a distinct kind.
- Help output no longer describes `mock_known_human` as a separate “KnownHuman checkpoint” preset.

---

### `src/daemon.rs` and `src/daemon/control_api.rs`

**Required changes:**

- If string checkpoint kinds from external/control inputs can contain `known_human`, normalize them to `CheckpointKind::Human`.
- Keep serialized outgoing kind as `"human"`.

**Acceptance criteria:**

- Existing inputs with `kind = "human"` keep working.
- Compatibility inputs with `kind = "known_human"` map to `CheckpointKind::Human` if such inputs reach this layer.
- No working-log entry serializes `kind = "known_human"`.

---

### `src/authorship/working_log.rs`

**Required changes:**

- Keep `CheckpointKind` limited to `Human`, `AiAgent`, and `AiTab`.
- Optionally allow `CheckpointKind::from_str("known_human")` to return `Some(CheckpointKind::Human)` only if historical or external data can reach this parser.
- Keep `CheckpointKind::to_str(CheckpointKind::Human)` returning `"human"`.

**Acceptance criteria:**

- No new `KnownHuman` variant exists.
- Any compatibility parse support is one-way: `known_human` input normalizes to `human`, but output remains `human`.

---

### `src/commands/diff.rs`

**Required changes:**

- Keep historical `h_` marker handling unless a separate migration explicitly removes it.
- Rename comments from “KnownHuman checkpoint” to “historical `h_` human attestation marker”.

**Acceptance criteria:**

- Diff output for historical notes with `h_` markers remains human-attributed.
- Comments no longer imply a current `KnownHuman` checkpoint kind.

---

### `src/authorship/virtual_attribution.rs`

**Required changes:**

- Keep skipping legacy `author_id == "human"` sentinel where current behavior requires it.
- Clarify comments around `h_` vs `human` sentinel.

**Acceptance criteria:**

- AI prompt-backed attribution behavior is unchanged.
- Legacy human sentinel behavior is unchanged.
- Historical `h_` behavior remains explicit and documented as compatibility.

---

### `src/authorship/attribution_tracker.rs`

**Required changes:**

- Audit comments around human/known-human handling.
- If code treats `h_` as AI-like due to `author_id != "human"`, decide explicitly whether that behavior is intentional compatibility or a bug.
- Do not silently change attribution dominance without tests.

**Acceptance criteria:**

- Comments match actual behavior.
- Any behavior change around `h_` requires a regression test showing the before/after intent.

---

### `src/authorship/authorship_log_serialization.rs`

**Required changes:**

- Fix comments that reference `metadata.humans` if the current schema does not actually contain that field.
- Clarify that `metadata.prompts` is for AI prompt-backed attestations, while historical human markers are compatibility-read behavior.

**Acceptance criteria:**

- Serialization comments match `AuthorshipMetadata` fields.
- No docs imply unsupported `metadata.humans` output.

---

### `tests/integration/formatting_non_substantial_ai_attribution.rs`

**Required changes:**

- Decide whether tests should continue using `mock_known_human` to cover compatibility alias behavior or switch to canonical `human` to cover core behavior.
- At least one test should explicitly cover `mock_known_human` as a compatibility alias if the alias remains supported.

**Acceptance criteria:**

- Core human behavior is tested through canonical human checkpoint usage.
- Compatibility alias behavior is tested intentionally, not accidentally.

---

### `tests/integration/stats.rs`

**Required changes:**

- Rename comments such as `known_human_accepted=0` to canonical human terminology.
- Ensure stats assertions still distinguish AI accepted lines from human additions.

**Acceptance criteria:**

- Stats tests no longer use stale known-human terminology unless they are explicitly testing compatibility aliases.

---

## Compatibility Strategy

### Keep accepting old commands

The following commands should remain valid:

```bash
git-ai checkpoint known_human --hook-input stdin
git-ai checkpoint known_human <pathspecs...>
git-ai checkpoint mock_known_human <pathspecs...>
```

But all must normalize to canonical human checkpoint behavior.

### Emit canonical data

Regardless of input alias:

- Working-log kind must serialize as `"human"`.
- Internal kind must be `CheckpointKind::Human`.
- No new `known_human` kind should appear in persisted checkpoint entries.

### Preserve historical reads

Old `h_` attribution markers remain a read compatibility concern. They should be documented and tested separately from checkpoint-kind normalization.

---

## Test Requirements

### CLI alias tests

Add or update tests to prove:

- `known_human` alias produces the same committed line attribution as canonical human checkpoint behavior.
- `mock_known_human` alias produces the same committed line attribution as canonical human checkpoint behavior.
- Alias use does not serialize `known_human` as a working-log kind.

### Hook-input tests

If `known_human --hook-input stdin` is still used by IDE integrations, test that a minimal valid payload is accepted and normalized.

### Historical note/read tests

Keep or add tests proving historical `h_` markers still render as human in diff/blame paths that support them.

### Regression tests

Run the existing tests that currently use `mock_known_human` after rewriting their intent:

```bash
task test TEST_FILTER=formatting_non_substantial_ai_attribution
task test TEST_FILTER=stats
```

Then run broader verification:

```bash
task build
task test
task lint
task fmt
```

---

## Acceptance Criteria

The change is complete when all of the following are true:

- [ ] No internal enum or persisted working-log kind named `known_human` exists.
- [ ] `known_human` and `mock_known_human` are explicit CLI compatibility aliases for human behavior.
- [ ] Alias inputs serialize as `"human"`, not `"known_human"`.
- [ ] Help text clearly labels old names as compatibility aliases.
- [ ] Tests intentionally cover both canonical human behavior and compatibility alias behavior.
- [ ] Historical `h_` marker handling is preserved or replaced only by an explicit, tested schema migration.
- [ ] Comments no longer imply a current `KnownHuman` checkpoint kind.
- [ ] `task build`, relevant tests, full `task test`, `task lint`, and `task fmt` pass.

---

## Risks and Non-Goals

### Risks

- **External integration breakage:** IDE plugins may still call `git-ai checkpoint known_human --hook-input stdin`. Removing the alias would break them.
- **Historical data loss:** Removing `h_` handling would make old human attestations display as unknown/no-data.
- **Silent behavior changes:** Changing attribution dominance around `h_` markers could alter blame/diff/stats output in old repositories.
- **Misleading tests:** Leaving tests named `mock_known_human` without an explicit compatibility purpose makes it unclear whether they test core behavior or legacy alias behavior.

### Non-goals

- Do not design a new authorship schema for human attestations in this change.
- Do not migrate all existing git notes or working logs.
- Do not remove old CLI aliases until all external integrations have migrated.
- Do not introduce `metadata.humans` unless handled as a separate schema/versioned format change.

---

## Recommended Implementation Order

### Task 1: Make CLI aliases explicit

**Files:**

- Modify: `src/commands/git_ai_handlers.rs`
- Test: relevant checkpoint integration tests

- [ ] Add explicit normalization for `known_human` and `mock_known_human`.
- [ ] Update help text to call them compatibility aliases.
- [ ] Add tests proving aliases serialize/output as canonical human behavior.

### Task 2: Normalize parser boundaries

**Files:**

- Modify: `src/authorship/working_log.rs`
- Modify: `src/daemon.rs`
- Modify: `src/daemon/control_api.rs`

- [ ] Add one-way parse compatibility only where external/historical strings can enter.
- [ ] Ensure output remains `"human"`.
- [ ] Add tests for parser normalization if the codebase already has parser unit tests.

### Task 3: Rename comments and terminology

**Files:**

- Modify: `src/commands/diff.rs`
- Modify: `src/authorship/virtual_attribution.rs`
- Modify: `src/authorship/attribution_tracker.rs`
- Modify: `src/authorship/authorship_log_serialization.rs`
- Modify: `tests/integration/stats.rs`

- [ ] Replace misleading “KnownHuman” language with canonical human / compatibility alias / historical `h_` marker terminology.
- [ ] Do not change behavior in this task unless tests already cover the intended result.

### Task 4: Clean test intent

**Files:**

- Modify: `tests/integration/formatting_non_substantial_ai_attribution.rs`

- [ ] Convert most tests to canonical human usage where they are testing core human behavior.
- [ ] Keep one focused compatibility alias test for `mock_known_human` if needed.
- [ ] Run targeted tests and full verification.

---

## Verification Commands

Run after implementation:

```bash
task build
task test TEST_FILTER=formatting_non_substantial_ai_attribution
task test TEST_FILTER=stats
task test
task lint
task fmt
```

Expected result: all commands pass, and no generated working-log entry uses `"known_human"` as a checkpoint kind.
