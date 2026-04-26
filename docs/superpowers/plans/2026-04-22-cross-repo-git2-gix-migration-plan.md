# Cross-Repo git2/gix Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the remaining easy Git CLI subprocess paths across the repository with the correct backend (`git2`, `gix`, or CLI hold) while preserving existing behavior and expanding migration coverage comprehensively.

**Architecture:** Execute migration in three lanes. Use `git2` for high-frequency local read-only object/ref/commit-graph queries, use `gix` for status/index/staged-content plumbing, and keep CLI for notes/raw-diff/merge/network/proxy behavior. Existing test cases are immutable: migration safety comes from adding new coverage and keeping old assertions green, not from weakening old tests.

**Tech Stack:** Rust 1.93.0 (edition 2024), `git2` 0.20.4, `gix-config` 0.53.0, `gix-index` 0.48.0, existing integration harness `tests/integration/main.rs`, existing repository requirements/spec documents.

---

## Source Documents

- `docs/superpowers/specs/2026-04-20-repository-git2-migration-requirements.md`
- `docs/git2-gix-对照表.md`
- `docs/superpowers/plans/2026-04-20-repository-git2-migration-checklist.md`
- `docs/superpowers/plans/2026-04-21-p1p2-git2-migration.md`

## Hard Constraints

- **Do not modify existing test cases.** Do not delete, weaken, reorder, or reinterpret current assertions to make a migration pass.
- **Migration coverage must be comprehensive.** Every migrated function needs explicit regression coverage for the current contract and edge cases.
- **New coverage must be additive.** Append new sections to existing comprehensive test files or create new dedicated migration test modules.
- **CLI hold-list is real scope control.** Do not opportunistically migrate notes writes, porcelain-sensitive blame output, raw diff formatting, merge strategy code, network operations, or git proxy behavior.
- **No semantic rewrites during backend swaps.** Keep public signatures, ordering, path handling, detached-HEAD behavior, and error semantics stable.

## Files

**Modify:**

- `src/authorship/rebase_authorship.rs`
- `src/commands/search.rs`
- `src/commands/blame.rs`
- `src/git/refs.rs`
- `src/git/sync_authorship.rs`
- `src/git/repository.rs`
- `src/git/status.rs`
- `src/git/diff_tree_to_tree.rs`
- `tests/integration/main.rs`
- `tests/integration/git_repository_comprehensive.rs`

**Create:**

- `tests/integration/git2_migration_aux_comprehensive.rs`
- `tests/integration/gix_status_index_comprehensive.rs`
- `tests/integration/gix_diff_tree_prototype.rs`

---

### Task 1: Establish migration guardrails and test routing

**Files:**
- Modify: `tests/integration/main.rs`
- Modify: `tests/integration/git_repository_comprehensive.rs` (append-only)
- Create: `tests/integration/git2_migration_aux_comprehensive.rs`
- Create: `tests/integration/gix_status_index_comprehensive.rs`
- Create: `tests/integration/gix_diff_tree_prototype.rs`

- [ ] **Step 1: Treat the requirements/spec docs as the routing source of truth**

Confirm that backend ownership stays aligned with the current docs:

- `git2`: object/ref/commit-graph reads
- `gix`: status/index/staged-content plumbing
- CLI: notes/raw-diff/merge/network/proxy paths

- [ ] **Step 2: Wire new migration-only test modules into the integration target**

Add new `mod ...;` entries in `tests/integration/main.rs` for the three new files without changing any existing module names or ordering assumptions.

- [ ] **Step 3: Freeze existing test semantics before code changes**

Implementation rule for all following tasks:

- existing test functions remain untouched
- existing assertions remain untouched
- new coverage goes into appended sections or new files only

- [ ] **Step 4: Run baseline focused integration commands before migration**

Run:
```bash
cargo test --package git-ai --test integration git_repository_comprehensive -- --nocapture
cargo test --package git-ai --test integration search -- --nocapture
cargo test --package git-ai --test integration blame_flags -- --nocapture
cargo test --package git-ai --test integration status_ignore -- --nocapture
cargo test --package git-ai --test integration diff_comprehensive -- --nocapture
cargo test --package git-ai --test integration rebase -- --nocapture
```

Expected: current baseline is recorded before any backend replacement begins.

---

### Task 2: Expand `repository.rs` git2 coverage without changing existing cases

**Files:**
- Modify: `tests/integration/git_repository_comprehensive.rs`

- [ ] **Step 1: Append new sections for the remaining `repository.rs` migration targets**

Add additive coverage for:

- `new_infer_refname()`
- `remote_head()`
- `upstream_remote()`
- `get_file_content()` edge cases that remain relevant to the not-yet-migrated path

- [ ] **Step 2: Cover the contract edges that are easy to break in a backend swap**

Include explicit cases for:

- attached vs detached `HEAD`
- remote `HEAD` symbolic ref resolution
- `branch.<name>.remote` config lookup behavior
- nested paths, spaces, and non-ASCII paths
- missing path and missing ref failures
- refname inference when multiple refs or no refs point at an object

- [ ] **Step 3: Keep this file append-only**

Do not rewrite or rename existing tests in `git_repository_comprehensive.rs`; only add new sections, helpers, and tests below the current coverage.

- [ ] **Step 4: Run the focused repository module**

Run:
```bash
cargo test --package git-ai --test integration git_repository_comprehensive -- --nocapture
```

Expected: old and new repository tests pass together.

---

### Task 3: Add comprehensive cross-repo git2 coverage outside `repository.rs`

**Files:**
- Create: `tests/integration/git2_migration_aux_comprehensive.rs`
- Modify: `tests/integration/main.rs`

- [ ] **Step 1: Add a dedicated comprehensive file for the non-`repository.rs` git2 lane**

Cover:

- `src/authorship/rebase_authorship.rs` → `walk_commits_to_base()`
- `src/commands/search.rs` → `search_by_commit_range()`
- `src/commands/blame.rs` → `resolve_blame_abbrev_shas_batched()`
- `src/git/refs.rs` → `ref_exists()`, `rev_parse()`, `copy_ref()`
- `src/git/sync_authorship.rs` → `get_current_branch()`

- [ ] **Step 2: Make the coverage contract-focused, not implementation-focused**

Include cases for:

- ancestry-path and topo-order behavior in merge-heavy histories
- empty and single-commit search ranges
- short SHA abbreviation stability and width handling
- missing refs, missing objects, and detached `HEAD`
- copy-ref semantics on existing and missing destinations

- [ ] **Step 3: Use existing `TestRepo` infrastructure instead of inventing a parallel harness**

The new test file should reuse the same repository fixture style already used in `search.rs`, `blame_flags.rs`, and other integration tests.

- [ ] **Step 4: Run the new comprehensive module**

Run:
```bash
cargo test --package git-ai --test integration git2_migration_aux_comprehensive -- --nocapture
```

Expected: the new aux coverage is green before implementation starts.

---

### Task 4: Implement the cross-repo git2 lane outside `repository.rs`

**Files:**
- Modify: `src/authorship/rebase_authorship.rs`
- Modify: `src/commands/search.rs`
- Modify: `src/commands/blame.rs`
- Modify: `src/git/refs.rs`
- Modify: `src/git/sync_authorship.rs`

- [ ] **Step 1: Replace subprocess-backed read paths only for the approved targets**

Implement only:

- `walk_commits_to_base()`
- `search_by_commit_range()`
- `resolve_blame_abbrev_shas_batched()`
- `ref_exists()`
- `rev_parse()`
- `copy_ref()`
- `get_current_branch()`

- [ ] **Step 2: Preserve contracts exactly**

Do not change:

- return types
- ordering guarantees
- missing-ref error behavior
- detached-HEAD behavior
- current short-SHA formatting assumptions

- [ ] **Step 3: Explicitly keep non-approved neighbors on CLI**

Do not fold in:

- `blame_hunks_for_ranges()`
- notes-related reads/writes beyond the targeted ref helpers
- network-backed auth/sync operations

- [ ] **Step 4: Run focused validation for this lane**

Run:
```bash
cargo test --package git-ai --test integration git2_migration_aux_comprehensive -- --nocapture
cargo test --package git-ai --test integration search -- --nocapture
cargo test --package git-ai --test integration blame_flags -- --nocapture
cargo test --package git-ai --test integration rebase -- --nocapture
```

Expected: the new comprehensive module and the legacy command-level tests all stay green.

---

### Task 5: Finish the remaining `repository.rs` git2 lane

**Files:**
- Modify: `src/git/repository.rs`
- Modify: `tests/integration/git_repository_comprehensive.rs` (append-only if gaps remain)

- [ ] **Step 1: Migrate the remaining approved `repository.rs` targets**

Implement only:

- `new_infer_refname()`
- `remotes()`
- `remotes_with_urls()`
- `remote_head()`
- `upstream_remote()`
- `get_file_content()`

- [ ] **Step 2: Reuse existing repository helpers instead of creating parallel abstractions**

Prefer:

- `self.open_git2()`
- existing config helpers
- existing path normalization and error wrapping patterns

- [ ] **Step 3: Keep `gix`-owned staged/index work out of this task**

Do not migrate `get_all_staged_files_content()` here; it belongs to the `gix` lane below.

- [ ] **Step 4: Run focused repository verification**

Run:
```bash
cargo test --package git-ai --test integration git_repository_comprehensive -- --nocapture
```

Expected: the repository comprehensive suite stays green with the remaining git2 conversions in place.

---

### Task 6: Add comprehensive `gix` status/index coverage

**Files:**
- Create: `tests/integration/gix_status_index_comprehensive.rs`
- Modify: `tests/integration/main.rs`

- [ ] **Step 1: Add a dedicated comprehensive file for the `gix` lane**

Cover:

- `src/git/status.rs` → `get_staged_filenames()`
- `src/git/status.rs` → `get_staged_and_unstaged_filenames()`
- `src/git/repo_state.rs` → branch / detached-HEAD metadata contract via `read_head_state_for_worktree()`
- `src/git/repository.rs` → `get_all_staged_files_content()`

- [ ] **Step 2: Make the status/index matrix exhaustive**

Include cases for:

- staged-only vs staged+unstaged mixes
- untracked handling and `skip_untracked`
- branch metadata and detached `HEAD`
- pathspec filtering and post-filter behavior
- non-ASCII pathspecs and paths with spaces
- conflicted index entries vs stage-0 / unconflicted entries
- binary vs text blob reads from the index

- [ ] **Step 3: Keep existing status-oriented tests untouched**

Do not rewrite `status_ignore.rs`, `e2big_post_filter.rs`, or other current status tests to fit the new implementation. The new file owns migration-specific backend coverage.

- [ ] **Step 4: Run the new gix coverage module**

Run:
```bash
cargo test --package git-ai --test integration gix_status_index_comprehensive -- --nocapture
```

Expected: the gix-lane contract is locked before implementation begins.

---

### Task 7: Implement the `gix` status/index lane

**Files:**
- Modify: `src/git/status.rs`
- Modify: `src/git/repository.rs`

- [ ] **Step 1: Replace only the approved status/index targets with `gix` plumbing**

Implement only:

- `get_staged_filenames()`
- `get_staged_and_unstaged_filenames()`
- `get_all_staged_files_content()`

- [ ] **Step 2: Preserve caller-facing shapes and filtering behavior**

Do not change:

- current JSON/status structure consumed by callers
- path normalization behavior
- post-filter behavior for large/non-ASCII pathspec sets
- stage-0-only semantics for staged-content reads

- [ ] **Step 3: Extend the current `gix` direction instead of fighting it**

Reuse existing `gix_index`-based patterns already present in `repository.rs`; only introduce additional `gix` plumbing that is necessary for the approved functions.

- [ ] **Step 4: Run focused validation for this lane**

Run:
```bash
cargo test --package git-ai --test integration gix_status_index_comprehensive -- --nocapture
cargo test --package git-ai --test integration status_ignore -- --nocapture
cargo test --package git-ai --test integration e2big_post_filter -- --nocapture
cargo test --package git-ai --test integration git_repository_comprehensive -- --nocapture
```

Expected: both the new gix-specific coverage and the older status/repository tests stay green.

---

### Task 8: Prototype `gix-diff` as a replace-or-retain decision

**Files:**
- Create: `tests/integration/gix_diff_tree_prototype.rs`
- Modify: `tests/integration/main.rs`
- Modify: `src/git/diff_tree_to_tree.rs`

- [ ] **Step 1: Add a dedicated prototype test harness for raw tree-diff equivalence**

Cover comparison of:

- delta ordering
- status letters
- old/new OIDs
- old/new modes
- rename/copy cases if currently supported
- path formatting assumptions of the current raw parser

- [ ] **Step 2: Build the `gix-diff` path as an adapter, not an immediate replacement**

Do not delete the CLI path during prototype work. The purpose of this task is to prove or disprove raw-contract equivalence.

- [ ] **Step 3: Make “keep CLI” a valid successful outcome**

If the adapter cannot reconstruct the current raw diff contract exactly enough, record the mismatch and retain the CLI implementation.

- [ ] **Step 4: Run prototype validation**

Run:
```bash
cargo test --package git-ai --test integration gix_diff_tree_prototype -- --nocapture
cargo test --package git-ai --test integration diff_comprehensive -- --nocapture
```

Expected: either the prototype proves parity, or it produces a documented reason to keep CLI.

---

### Task 9: Final verification and CLI hold-line audit

**Files:**
- Inspect all touched source and test files

- [ ] **Step 1: Run the full integration target**

Run:
```bash
cargo test --package git-ai --test integration -- --nocapture
```

Expected: all integration modules pass together.

- [ ] **Step 2: Run the full repository suite**

Run:
```bash
cargo test
```

Expected: all tests pass.

- [ ] **Step 3: Run lint and format verification**

Run:
```bash
cargo clippy
cargo fmt -- --check
```

Expected: zero lint failures and clean formatting.

- [ ] **Step 4: Audit the CLI hold-list explicitly**

Confirm no unintended changes were made to:

- `src/git/repository.rs` → `diff_*` family, `list_commit_files()`, `merge_trees_favor_ours()`
- `src/git/refs.rs` → `notes_add_batch()`, `notes_add_blob_batch()`, `merge_notes_from_ref()`, `fallback_merge_notes()`, `grep_ai_notes()`
- `src/commands/log.rs` → `handle_log()`
- `src/commands/git_handlers.rs` → `handle_git()` / `run_git_with_hooks()`
- `src/commands/blame.rs` → `blame_hunks_for_ranges()`
- `src/git/sync_authorship.rs` → `fetch_authorship_notes()`, `push_authorship_notes()`

- [ ] **Step 5: Audit the test policy explicitly**

Confirm both of these are true:

- no existing test case was modified to accommodate migration
- comprehensive additive coverage now exists for every migrated function

---

## Recommended PR Breakdown

1. **PR1 — Guardrails + repository coverage expansion**
   - Task 1
   - Task 2

2. **PR2 — Auxiliary git2 lane**
   - Task 3
   - Task 4

3. **PR3 — Remaining `repository.rs` git2 lane**
   - Task 5

4. **PR4 — `gix` status/index lane**
   - Task 6
   - Task 7

5. **PR5 — `gix-diff` prototype / retain-CLI decision**
   - Task 8

6. **PR6 — Final verification-only sweep if needed**
   - Task 9

## Completion Criteria

- The `git2` lane is complete for the approved easy targets.
- The `gix` lane is complete for status/index/staged-content targets.
- The CLI hold-list remains untouched unless a later approved plan changes scope.
- Existing tests remain unmodified in semantics.
- Migration coverage is comprehensive and additive.
- Full test, lint, and format verification passes.
