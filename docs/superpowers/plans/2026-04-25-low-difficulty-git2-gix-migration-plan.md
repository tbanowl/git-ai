# Low-Difficulty git2/gix Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate the remaining low-difficulty read-only Git CLI paths to the fastest correct in-process backend (`git2`, `gix`, or `git2 + gix`) while preserving existing behavior and strengthening regression coverage.

**Architecture:** Execute the work in three batches following the approved design. Batch 1 handles direct metadata/resolve reads, Batch 2 handles commit-graph/history/context reads, and Batch 3 handles detached/worktree-sensitive repo-context reads. Backend selection is performance-first: use `git2` for object/ref/graph queries when it is the best fit, use `gix` for path/discover/worktree semantics when it is the better fit, and allow mixed `git2 + gix` implementations when that preserves behavior with lower overhead than either library alone.

**Tech Stack:** Rust 1.93.0 (edition 2024), `git2`, existing `gix` / `gix_index` infrastructure, integration tests under `tests/integration/`, approved spec `docs/superpowers/specs/2026-04-25-low-difficulty-git2-migration-design.md`.

---

## Source Documents

- `docs/superpowers/specs/2026-04-25-low-difficulty-git2-migration-design.md`
- `docs/git2-gix-对照表.md`
- `docs/superpowers/plans/2026-04-22-cross-repo-git2-gix-migration-plan.md`
- `docs/superpowers/plans/2026-04-24-status-gix-migration-plan.md`

## Hard Constraints

- **No implementation in this planning phase.** This file only defines execution.
- **Performance-first backend selection.** Do not force a single library across all targets.
- **Only low-difficulty, read-only paths are in scope.** No notes writes, raw diff formatting, status porcelain replacement, network operations, or write-side Git plumbing.
- **Behavioral parity is mandatory.** Keep signatures, ordering, detached-HEAD handling, message formatting, and error behavior stable.
- **Every migrated function must carry source-command comments.** Add `// Migrated from: git ...` and optionally `// Backend: git2`, `// Backend: gix`, or `// Backend: git2 + gix (performance-first)`.
- **Tests must verify behavior, not implementation detail.** Avoid asserting that a subprocess was or was not spawned.

## File structure map

### Source files likely to change

- `src/commands/diff.rs`
- `src/api/client.rs`
- `src/commands/checkpoint_agent/bash_tool.rs`
- `src/commands/prompts_db.rs`
- `src/commands/continue_session.rs`
- `src/daemon.rs`
- `src/daemon/git_backend.rs`
- `src/commands/hooks/rebase_hooks.rs`
- `src/commands/hooks/cherry_pick_hooks.rs`
- `src/commands/hooks/update_ref_hooks.rs`
- `src/commands/hooks/reset_hooks.rs`

### Test files likely to change

- `tests/integration/git2_migration_aux_comprehensive.rs`
- `tests/integration/prompts_db_test.rs`
- `tests/integration/bash_tool_conformance.rs`
- `tests/integration/main.rs`
- Existing `continue_session` / daemon-related integration test files if found during implementation

### Planning note on test placement

- Prefer appending to `tests/integration/git2_migration_aux_comprehensive.rs` for cross-module migration coverage.
- Prefer reusing `prompts_db_test.rs` and `bash_tool_conformance.rs` for behavior-level regressions already near the relevant code.
- If no suitable `continue_session` or `daemon/git_backend` coverage file exists, create one focused integration file instead of overloading unrelated suites.

---

## Batch routing table

| Batch | Function | Preferred backend | Test home | Verification | Primary risk |
|---|---|---|---|---|---|
| 1 | `src/commands/diff.rs::resolve_commit()` | `git2` | `git2_migration_aux_comprehensive.rs` | compare result to `git rev-parse` | invalid rev / non-commit peel semantics |
| 1 | `src/api/client.rs::resolve_git_identity()` | `git2` or `gix-config` | new/additive API-client-focused integration coverage | compare formatted identity to current config/env behavior | env/config precedence drift |
| 1 | `src/commands/checkpoint_agent/bash_tool.rs::get_git_dir()` | `gix` or `git2 + gix` | `bash_tool_conformance.rs` | compare resolved git dir in normal repo and worktree | relative vs absolute `.git` / worktree layout |
| 1 | `src/commands/prompts_db.rs::reachable_commits()` | `git2` | `git2_migration_aux_comprehensive.rs`, `prompts_db_test.rs` | compare reachable SHA set to `git rev-list --all` | walk flags / hidden refs mismatch |
| 1 | `src/commands/prompts_db.rs::commit_dates_for()` | `git2` | `git2_migration_aux_comprehensive.rs`, `prompts_db_test.rs` | compare SHA->timestamp map to current behavior | committer-time vs author-time mismatch |
| 2 | `src/commands/continue_session.rs::CommitInfo::from_commit_sha()` | `git2` | new/existing continue_session integration tests | compare subject/body/date/author values to current Git output | multiline message/body formatting drift |
| 2 | `src/commands/continue_session.rs::get_git_status_info()` | `git2` or `git2 + gix` | same as above | branch string + recent commit ordering | detached HEAD / recent walk order |
| 2 | `src/daemon.rs::is_ancestor_commit()` | `git2` | `git2_migration_aux_comprehensive.rs` or daemon-focused tests | ancestor/non-ancestor/equal cases | boolean semantics on missing objects |
| 2 | `src/commands/hooks/rebase_hooks.rs::walk_first_parent_commits()` | `git2` | `git2_migration_aux_comprehensive.rs` | compare ordered list to `git rev-list --first-parent --topo-order` | first-parent order drift |
| 2 | `src/commands/hooks/rebase_hooks.rs::is_ancestor()` | `git2` | same as above | ancestor truth table | missing-object handling |
| 2 | `src/commands/hooks/cherry_pick_hooks.rs::expand_commit_range()` | `git2` | `git2_migration_aux_comprehensive.rs` | compare ordered list to `git rev-list --reverse` | range parsing / reverse ordering |
| 2 | `src/commands/hooks/cherry_pick_hooks.rs::resolve_commit_sha()` | `git2` | `git2_migration_aux_comprehensive.rs` | compare SHA to `git rev-parse` | symbolic ref / annotated tag behavior |
| 2 | `src/commands/hooks/update_ref_hooks.rs::is_ancestor()` | `git2` | `git2_migration_aux_comprehensive.rs` | ancestor truth table | hidden divergence from shared helper semantics |
| 2 | `src/commands/hooks/reset_hooks.rs::is_ancestor()` | `git2` | `git2_migration_aux_comprehensive.rs` | ancestor truth table | hidden divergence from shared helper semantics |
| 3 | `src/daemon/git_backend.rs::repo_context()` | `gix` or `git2 + gix` | daemon / repo-context integration tests | attached/detached/worktree branch metadata | detached HEAD / worktree common-dir semantics |
| 3 | `src/daemon/git_backend.rs::rev_parse_head()` | `git2` | same as above | compare HEAD SHA to `git rev-parse --verify HEAD` | worktree/head peel semantics |

---

### Task 1: Lock Batch 1 behavior before backend swaps

**Files:**
- Modify: `tests/integration/git2_migration_aux_comprehensive.rs`
- Modify: `tests/integration/prompts_db_test.rs`
- Modify: `tests/integration/bash_tool_conformance.rs`
- Modify: `tests/integration/main.rs` (only if a new test module is needed)

- [ ] **Step 1: Add additive tests for `resolve_commit()` parity**

Extend `tests/integration/git2_migration_aux_comprehensive.rs` with tests that compare the function output to `git rev-parse` for:

- `HEAD`
- full commit SHA
- branch name
- invalid revspec error

Expected assertion shape:

```rust
assert_eq!(actual, repo.git(&["rev-parse", rev]).unwrap().trim());
```

- [ ] **Step 2: Add additive tests for `reachable_commits()` and `commit_dates_for()`**

Add tests that:

- build a small multi-commit history
- compare `reachable_commits()` against `git rev-list --all`
- compare `commit_dates_for()` against `git show -s --format=%H %ct <sha>...`

- [ ] **Step 3: Strengthen `prompts_db_test.rs` only at the behavior boundary**

Add assertions that still pass whether the implementation is CLI, `git2`, or `gix`, for example:

- commits filtered by `--since` remain visible as expected
- prompt rows linked to reachable commits remain present
- orphaned or missing commit handling stays stable if that behavior already exists

- [ ] **Step 4: Additive `get_git_dir()` behavior tests in `bash_tool_conformance.rs`**

Cover:

- regular repository `.git` resolution
- worktree `.git` indirection / linked worktree layout
- returned path usability when joined with `ai/bash_snapshots`

- [ ] **Step 5: Run focused Batch 1 tests before implementation**

Run:

```bash
task test TEST_FILTER=git2_migration_aux_comprehensive
task test TEST_FILTER=prompts_db_test
task test TEST_FILTER=bash_tool_conformance
```

Expected: PASS. This becomes the pre-migration behavioral baseline.

---

### Task 2: Implement Batch 1 with performance-first backend choices

**Files:**
- Modify: `src/commands/diff.rs`
- Modify: `src/api/client.rs`
- Modify: `src/commands/checkpoint_agent/bash_tool.rs`
- Modify: `src/commands/prompts_db.rs`

- [ ] **Step 1: Migrate `resolve_commit()` to `git2`**

Add comments:

```rust
// Migrated from: git rev-parse <rev>
// Backend: git2
```

Implementation constraint: preserve current error path when the rev cannot be resolved to a commit SHA.

- [ ] **Step 2: Migrate `resolve_git_identity()` using the fastest correct config path**

Backend preference:

- use existing `git2`-based identity resolution if it preserves Git precedence correctly
- if `gix-config` makes precedence handling simpler and equally verifiable, switch to it

Add comments:

```rust
// Migrated from: git var GIT_COMMITTER_IDENT
// Backend: git2   // or gix, depending on final route
```

- [ ] **Step 3: Migrate `get_git_dir()` with `gix` or `git2 + gix`**

Backend preference:

- prefer `gix` for discover/open and repository path semantics
- allow `git2 + gix` if worktree/common-dir handling is measurably safer

Add comments:

```rust
// Migrated from: git -C <repo> rev-parse --git-dir
// Backend: gix   // or git2 + gix (performance-first)
```

- [ ] **Step 4: Migrate `reachable_commits()` and `commit_dates_for()` to `git2`**

Add comments:

```rust
// Migrated from: git rev-list --all
// Backend: git2
```

```rust
// Migrated from: git show -s --format=%H %ct <sha>...
// Backend: git2
```

- [ ] **Step 5: Run focused Batch 1 verification**

Run:

```bash
task test TEST_FILTER=git2_migration_aux_comprehensive
task test TEST_FILTER=prompts_db_test
task test TEST_FILTER=bash_tool_conformance
task build
```

Expected: PASS. No behavior regressions in Batch 1 surfaces.

---

### Task 3: Lock Batch 2 behavior before backend swaps

**Files:**
- Modify: `tests/integration/git2_migration_aux_comprehensive.rs`
- Modify: existing continue-session or daemon integration test files if present
- Create: a new focused integration file only if no suitable test home exists

- [ ] **Step 1: Add additive history/ancestor parity tests**

In `git2_migration_aux_comprehensive.rs`, add coverage for:

- `expand_commit_range()` vs `git rev-list --reverse`
- `resolve_commit_sha()` vs `git rev-parse`
- `walk_first_parent_commits()` vs `git rev-list --first-parent --topo-order --max-count=N`
- all `is_ancestor*()` variants with equal, ancestor, non-ancestor, and missing-commit cases

- [ ] **Step 2: Add continue-session behavior tests**

Cover:

- multiline commit message subject/body extraction
- current branch rendering when attached
- recent commits listing length and ordering
- detached HEAD fallback behavior

- [ ] **Step 3: Keep these tests behavior-only**

Do not assert on internal helper names or whether a subprocess was called. Compare outputs and observable fields only.

- [ ] **Step 4: Run focused Batch 2 tests before implementation**

Run:

```bash
task test TEST_FILTER=git2_migration_aux_comprehensive
task test TEST_FILTER=continue_session
task test TEST_FILTER=daemon
```

Expected: PASS or, if filter names differ, an updated equivalent command set recorded before implementation.

---

### Task 4: Implement Batch 2 with `git2`-first routing

**Files:**
- Modify: `src/commands/continue_session.rs`
- Modify: `src/daemon.rs`
- Modify: `src/commands/hooks/rebase_hooks.rs`
- Modify: `src/commands/hooks/cherry_pick_hooks.rs`
- Modify: `src/commands/hooks/update_ref_hooks.rs`
- Modify: `src/commands/hooks/reset_hooks.rs`

- [ ] **Step 1: Migrate commit metadata/history reads in `continue_session.rs`**

Preferred backend: `git2`.

Comments required:

```rust
// Migrated from:
// - git log -1 --format=%H|||%an|||%ai|||%s <sha>
// - git log -1 --format=%B <sha>
// Backend: git2
```

```rust
// Migrated from:
// - git branch --show-current
// - git log --oneline -5
// Backend: git2   // or git2 + gix if branch-name handling needs a mixed route
```

- [ ] **Step 2: Migrate ancestor checks to `git2`**

Targets:

- `daemon.rs::is_ancestor_commit()`
- `rebase_hooks.rs::is_ancestor()`
- `update_ref_hooks.rs::is_ancestor()`
- `reset_hooks.rs::is_ancestor()`

Use a shared low-level parity rule: preserve current boolean semantics for valid and invalid objects.

- [ ] **Step 3: Migrate `walk_first_parent_commits()` and `expand_commit_range()`**

Preferred backend: `git2 revwalk()`.

Guardrails:

- preserve existing ordering exactly
- do not accidentally include extra branch history
- keep current empty-range behavior

- [ ] **Step 4: Migrate `resolve_commit_sha()`**

Preferred backend: `git2 revparse_single()` + peel-to-commit.

- [ ] **Step 5: Run focused Batch 2 verification**

Run:

```bash
task test TEST_FILTER=git2_migration_aux_comprehensive
task test TEST_FILTER=continue_session
task test TEST_FILTER=rebase
task test TEST_FILTER=daemon
task build
```

Expected: PASS. Ordering, ancestor checks, and message formatting remain stable.

---

### Task 5: Lock Batch 3 behavior before backend swaps

**Files:**
- Modify: daemon / repo-context integration tests if present
- Create: a new focused repo-context integration test only if needed

- [ ] **Step 1: Add `repo_context()` parity tests**

Cover:

- attached branch name in a regular worktree
- detached HEAD reports `detached=true` and no branch
- linked worktree still reports the right branch/head metadata

- [ ] **Step 2: Add `rev_parse_head()` parity tests**

Compare returned SHA against `git rev-parse --verify HEAD` in:

- regular repository
- linked worktree
- detached HEAD

- [ ] **Step 3: Run focused Batch 3 tests before implementation**

Run the exact daemon / repo-context test target(s) that cover the above scenarios and record the passing baseline.

---

### Task 6: Implement Batch 3 with `gix`-aware repo-context routing

**Files:**
- Modify: `src/daemon/git_backend.rs`

- [ ] **Step 1: Migrate `repo_context()` with `gix`-first routing**

Backend preference:

- prefer `gix` for head/refname and worktree metadata discovery
- allow `git2 + gix` if detached/worktree semantics are more robust that way

Comments required:

```rust
// Migrated from: git symbolic-ref --quiet --short HEAD
// Backend: gix   // or git2 + gix (performance-first)
```

- [ ] **Step 2: Migrate `rev_parse_head()` with `git2` unless tests prove a better mixed route**

Comments required:

```rust
// Migrated from: git rev-parse --verify HEAD
// Backend: git2
```

- [ ] **Step 3: Run focused Batch 3 verification**

Run the repo-context / daemon tests identified in Task 5 plus:

```bash
task build
```

Expected: PASS. Attached/detached/worktree semantics remain identical.

---

### Task 7: Final verification and diff review

**Files:**
- Review all changed source and test files

- [ ] **Step 1: Run the aggregated targeted verification set**

Run:

```bash
task test TEST_FILTER=git2_migration_aux_comprehensive
task test TEST_FILTER=prompts_db_test
task test TEST_FILTER=bash_tool_conformance
task test TEST_FILTER=continue_session
task test TEST_FILTER=daemon
task test TEST_FILTER=rebase
task build
```

Expected: PASS. If any filter name is unavailable, replace it with the exact integration target covering the same behavior and record that substitution in the implementation notes.

- [ ] **Step 2: Review each migrated function for mandatory comments**

Every migrated function must contain:

- `Migrated from: git ...`
- `Backend: git2`, `Backend: gix`, or `Backend: git2 + gix (performance-first)`

- [ ] **Step 3: Review for accidental scope creep**

Confirm that no notes, diff-format, status-porcelain, network, or write-side Git paths were opportunistically migrated.

- [ ] **Step 4: Prepare a completion summary**

The execution summary must list:

- each migrated function
- chosen backend
- test file that covers it
- any function deferred due to higher-than-expected complexity

---

## Self-review against spec

### Spec coverage

- Batch 1 / 2 / 3 routing from the spec is preserved.
- Each target function has a preferred backend, test home, verification mode, and risk called out.
- Comment requirements and performance-first routing are explicitly included.
- Low-difficulty scope remains limited to read-only, non-notes, non-network, non-porcelain-sensitive paths.

### Placeholder scan

- No `TODO`, `TBD`, or “implement later” placeholders remain.
- Commands and verification expectations are concrete, though exact test filter substitutions may be needed if the current suite uses different names.

### Type / naming consistency

- Plan uses the exact source function names from the approved spec.
- Backend labels consistently use `git2`, `gix`, or `git2 + gix`.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-04-25-low-difficulty-git2-gix-migration-plan.md`. Two execution options:

**1. Subagent-Driven (recommended)** - I dispatch a fresh subagent per task, review between tasks, fast iteration

**2. Inline Execution** - Execute tasks in this session using executing-plans, batch execution with checkpoints

**Which approach?**
