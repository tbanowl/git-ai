# status.rs gix Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the remaining `git status --porcelain=v2 -z` path inside `Repository::status()` with an in-process implementation while preserving `StatusEntry` semantics exactly.

**Architecture:** Keep `Repository::status()` and the public status types stable, but split the internals into three layers inside `src/git/status.rs`: a raw collection layer, a normalization/filter layer, and a final `StatusEntry` assembly layer. Prefer `gix`-style/index-plumbing reads for raw state collection, and allow narrow `git2` fallback only where rename/copy detection would otherwise force a worse design.

**Tech Stack:** Rust 2024, `gix_index`, existing `git2`, repository integration tests under `tests/integration/`

---

## File structure map

### Files to modify

- `src/git/status.rs`
  - Keep `StatusCode`, `EntryKind`, and `StatusEntry` unchanged.
  - Add internal raw-status structs and helpers.
  - Replace the CLI-backed `Repository::status()` body with the layered implementation.
  - Keep post-filter/NFC/pathspec union semantics in this file.
- `tests/integration/gix_status_index_comprehensive.rs`
  - Extend semantic coverage for `Repository::status()` parity, especially unmerged behavior and rename/copy expectations.
- `tests/integration/e2big_post_filter.rs`
  - Preserve the large-pathspec and `orig_path` post-filter contract while the implementation changes underneath.

### Files to inspect while implementing

- `docs/superpowers/specs/2026-04-24-status-gix-migration-design.md`
- `docs/git2-gix-对照表.md`
- `src/git/status.rs`
- `src/git/repo_state.rs` (reference only; do not fold into this change)

### Files not to modify unless absolutely necessary

- `src/git/repo_state.rs`
- `src/git/repository.rs`
- `src/commands/status.rs`

---

## Task 1: Lock the current status() contract with explicit parity-focused tests

**Files:**
- Modify: `tests/integration/gix_status_index_comprehensive.rs`
- Modify: `tests/integration/e2big_post_filter.rs`
- Inspect: `src/git/status.rs`

- [ ] **Step 1: Add one focused helper for entry lookup + path assertions where missing**

Add or reuse helpers so later tests stay short and compare the observable contract only.

```rust
fn status_entry_by_path<'a>(entries: &'a [StatusEntry], path: &str) -> &'a StatusEntry {
    entries
        .iter()
        .find(|entry| entry.path == path)
        .unwrap_or_else(|| panic!("missing status entry for {path}"))
}

fn sorted_entry_paths(entries: &[StatusEntry]) -> Vec<String> {
    let mut paths: Vec<String> = entries.iter().map(|entry| entry.path.clone()).collect();
    paths.sort();
    paths
}
```

- [ ] **Step 2: Add a regression test for full-scan behavior when pathspecs are absent and nothing is staged**

Add this test to `tests/integration/gix_status_index_comprehensive.rs`.

```rust
#[test]
fn gix_status_index_comprehensive_status_without_pathspecs_still_reports_pure_unstaged_changes() {
    let repo = TestRepo::new();

    write_file(&repo, "tracked.txt", "seed\n");
    repo.stage_all_and_commit("initial").unwrap();

    write_file(&repo, "tracked.txt", "seed\nunstaged\n");

    let repository = open_repo(&repo);
    let entries = repository.status(None, true).unwrap();

    let tracked = status_entry_by_path(&entries, "tracked.txt");
    assert_eq!(tracked.kind, EntryKind::Ordinary);
    assert_eq!(tracked.staged, StatusCode::Unmodified);
    assert_eq!(tracked.unstaged, StatusCode::Modified);
}
```

- [ ] **Step 3: Add a regression test for staged deletion semantics**

Add this test to `tests/integration/gix_status_index_comprehensive.rs`.

```rust
#[test]
fn gix_status_index_comprehensive_status_reports_staged_deletions() {
    let repo = TestRepo::new();

    write_file(&repo, "gone.txt", "seed\n");
    repo.stage_all_and_commit("initial").unwrap();

    std::fs::remove_file(repo.path().join("gone.txt")).unwrap();
    repo.git_og(&["add", "gone.txt"]).unwrap();

    let repository = open_repo(&repo);
    let entries = repository.status(None, true).unwrap();

    let gone = status_entry_by_path(&entries, "gone.txt");
    assert_eq!(gone.kind, EntryKind::Ordinary);
    assert_eq!(gone.staged, StatusCode::Deleted);
    assert_eq!(gone.unstaged, StatusCode::Unmodified);
}
```

- [ ] **Step 4: Tighten rename/copy post-filter expectations around `orig_path`**

Keep the existing large-pathspec tests in `tests/integration/e2big_post_filter.rs`, and add one assertion that checks both the path and entry kind.

```rust
let entry = result.iter().find(|e| e.path == "new.txt").unwrap();
assert_eq!(entry.kind, EntryKind::Rename);
assert_eq!(entry.orig_path.as_deref(), Some("old.txt"));
```

- [ ] **Step 5: Add a regression test that unresolved conflicts surface as `EntryKind::Unmerged`**

Add this test to `tests/integration/gix_status_index_comprehensive.rs`.

```rust
#[test]
fn gix_status_index_comprehensive_status_reports_unmerged_entries() {
    let repo = TestRepo::new();

    write_file(&repo, "conflicted.txt", "base\n");
    repo.stage_all_and_commit("initial").unwrap();

    repo.git_og(&["checkout", "-b", "feature"]).unwrap();
    write_file(&repo, "conflicted.txt", "feature\n");
    repo.stage_all_and_commit("feature change").unwrap();

    repo.git_og(&["checkout", "-"]).unwrap();
    write_file(&repo, "conflicted.txt", "main\n");
    repo.stage_all_and_commit("main change").unwrap();

    let merge = repo.git_og(&["merge", "feature"]);
    assert!(merge.is_err(), "merge should leave conflict stages behind");

    let repository = open_repo(&repo);
    let entries = repository.status(None, false).unwrap();
    let conflicted = status_entry_by_path(&entries, "conflicted.txt");

    assert_eq!(conflicted.kind, EntryKind::Unmerged);
    assert!(
        conflicted.staged == StatusCode::Unmerged || conflicted.unstaged == StatusCode::Unmerged
    );
}
```

- [ ] **Step 6: Run the focused status integration tests and capture the baseline**

Run:

```bash
task test TEST_FILTER=gix_status_index_comprehensive
task test TEST_FILTER=e2big_post_filter
```

Expected: PASS. These tests are the baseline contract that the new implementation must continue to satisfy.

- [ ] **Step 7: Commit the contract-locking tests**

```bash
git add tests/integration/gix_status_index_comprehensive.rs tests/integration/e2big_post_filter.rs
git commit -m "test: lock status() parity semantics before gix migration"
```

---

## Task 2: Introduce internal raw-status and normalization layers inside src/git/status.rs

**Files:**
- Modify: `src/git/status.rs`
- Test: `tests/integration/gix_status_index_comprehensive.rs`

- [ ] **Step 1: Add internal structs for raw collection and normalized filtering**

Add these private types near the existing status types in `src/git/status.rs`.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CollectedKind {
    Ordinary,
    Rename,
    Copy,
    Unmerged,
    Untracked,
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawSideStatus {
    Unmodified,
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    Unmerged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CollectedStatusRecord {
    path: String,
    staged: RawSideStatus,
    unstaged: RawSideStatus,
    kind: CollectedKind,
    orig_path: Option<String>,
}
```

- [ ] **Step 2: Add explicit conversion helpers from raw records to public status types**

Add these private helpers in `src/git/status.rs`.

```rust
fn public_status_code(value: RawSideStatus) -> StatusCode {
    match value {
        RawSideStatus::Unmodified => StatusCode::Unmodified,
        RawSideStatus::Modified => StatusCode::Modified,
        RawSideStatus::Added => StatusCode::Added,
        RawSideStatus::Deleted => StatusCode::Deleted,
        RawSideStatus::Renamed => StatusCode::Renamed,
        RawSideStatus::Copied => StatusCode::Copied,
        RawSideStatus::Unmerged => StatusCode::Unmerged,
    }
}

fn public_entry_kind(value: CollectedKind) -> EntryKind {
    match value {
        CollectedKind::Ordinary => EntryKind::Ordinary,
        CollectedKind::Rename => EntryKind::Rename,
        CollectedKind::Copy => EntryKind::Copy,
        CollectedKind::Unmerged => EntryKind::Unmerged,
        CollectedKind::Untracked => EntryKind::Untracked,
        CollectedKind::Ignored => EntryKind::Ignored,
    }
}

fn assemble_status_entry(record: CollectedStatusRecord) -> StatusEntry {
    StatusEntry {
        path: record.path,
        staged: public_status_code(record.staged),
        unstaged: match record.kind {
            CollectedKind::Untracked => StatusCode::Untracked,
            CollectedKind::Ignored => StatusCode::Ignored,
            _ => public_status_code(record.unstaged),
        },
        kind: public_entry_kind(record.kind),
        orig_path: record.orig_path,
    }
}
```

- [ ] **Step 3: Extract the pathspec decision logic out of `Repository::status()` into helpers**

Add a dedicated decision helper so pathspec union/full-scan/post-filter rules remain explicit and testable.

```rust
#[derive(Debug, Clone)]
struct StatusPathspecPlan {
    combined_pathspecs: HashSet<String>,
    should_full_scan: bool,
    needs_post_filter: bool,
}

fn build_status_pathspec_plan(
    staged_filenames: HashSet<String>,
    pathspecs: Option<&HashSet<String>>,
) -> StatusPathspecPlan {
    let combined_pathspecs: HashSet<String> = if let Some(paths) = pathspecs {
        staged_filenames.union(paths).cloned().collect()
    } else {
        staged_filenames
    };

    let should_full_scan = pathspecs.is_none() && combined_pathspecs.is_empty();
    let has_non_ascii = combined_pathspecs.iter().any(|path| !path.is_ascii());
    let needs_post_filter =
        !should_full_scan && (combined_pathspecs.len() > MAX_PATHSPEC_ARGS || has_non_ascii);

    StatusPathspecPlan {
        combined_pathspecs,
        should_full_scan,
        needs_post_filter,
    }
}
```

- [ ] **Step 4: Extract the existing retain logic into a helper that works on `StatusEntry` slices**

Keep the same post-filter contract, including `orig_path` matches.

```rust
fn post_filter_status_entries(
    entries: &mut Vec<StatusEntry>,
    combined_pathspecs: &HashSet<String>,
) {
    let nfc_pathspecs: HashSet<String> = combined_pathspecs
        .iter()
        .map(|path| nfc_path(path.clone()))
        .collect();

    entries.retain(|entry| {
        nfc_pathspecs.contains(&entry.path)
            || entry
                .orig_path
                .as_ref()
                .is_some_and(|orig_path| nfc_pathspecs.contains(orig_path))
    });
}
```

- [ ] **Step 5: Wire `Repository::status()` through the new helpers but keep the CLI parser as the temporary data source**

At this stage, the behavior should not change; only the structure changes. Replace the body with the helper-based flow.

```rust
pub fn status(
    &self,
    pathspecs: Option<&HashSet<String>>,
    skip_untracked: bool,
) -> Result<Vec<StatusEntry>, GitAiError> {
    let staged_filenames = self.get_staged_filenames()?;
    let plan = build_status_pathspec_plan(staged_filenames, pathspecs);

    if plan.combined_pathspecs.is_empty() && !plan.should_full_scan {
        return Ok(Vec::new());
    }

    let output = run_status_cli(self, &plan, skip_untracked)?;
    let mut entries = parse_porcelain_v2(&output)?;

    if plan.needs_post_filter {
        post_filter_status_entries(&mut entries, &plan.combined_pathspecs);
    }

    Ok(entries)
}
```

- [ ] **Step 6: Run the focused tests to confirm this refactor is behavior-preserving**

Run:

```bash
task test TEST_FILTER=gix_status_index_comprehensive
task test TEST_FILTER=e2big_post_filter
```

Expected: PASS. If this fails, fix the refactor before introducing any non-CLI collection logic.

- [ ] **Step 7: Commit the internal layering refactor**

```bash
git add src/git/status.rs tests/integration/gix_status_index_comprehensive.rs tests/integration/e2big_post_filter.rs
git commit -m "refactor: layer status() pathspec and assembly flow"
```

---

## Task 3: Replace the CLI data source with in-process raw status collection

**Files:**
- Modify: `src/git/status.rs`
- Test: `tests/integration/gix_status_index_comprehensive.rs`
- Test: `tests/integration/e2big_post_filter.rs`

- [ ] **Step 1: Add a private collection entry point that returns raw records instead of porcelain text**

Add this private function in `src/git/status.rs`.

```rust
fn collect_status_records(
    repo: &Repository,
    plan: &StatusPathspecPlan,
    skip_untracked: bool,
) -> Result<Vec<CollectedStatusRecord>, GitAiError> {
    let mut records = collect_tracked_records(repo)?;

    if !skip_untracked {
        records.extend(collect_untracked_records(repo)?);
    }

    records.extend(collect_ignored_records(repo)?);

    if !plan.should_full_scan && !plan.needs_post_filter {
        records.retain(|record| plan.combined_pathspecs.contains(&record.path));
    }

    Ok(records)
}
```

- [ ] **Step 2: Implement tracked-record collection around HEAD/index/worktree state, not parser text**

Introduce a single merge point that computes `HEAD -> index` and `index -> worktree` facts before final assembly.

```rust
fn collect_tracked_records(repo: &Repository) -> Result<Vec<CollectedStatusRecord>, GitAiError> {
    let tracked_facts = collect_head_index_worktree_facts(repo)?;

    let mut records = Vec::new();
    for fact in tracked_facts {
        records.push(CollectedStatusRecord {
            path: fact.path,
            staged: fact.staged,
            unstaged: fact.unstaged,
            kind: fact.kind,
            orig_path: fact.orig_path,
        });
    }

    Ok(records)
}
```

The exact helper names may differ, but the code must keep all tracked-state derivation inside `src/git/status.rs` and must not reintroduce porcelain text as an intermediate format.

- [ ] **Step 3: Use `gix`/index plumbing for index-driven facts, and keep `git2` fallback narrow if rename/copy detection needs it**

The implementation code should follow this shape.

```rust
fn collect_head_index_worktree_facts(repo: &Repository) -> Result<Vec<TrackedFact>, GitAiError> {
    let index_facts = collect_index_facts(repo)?;
    let head_facts = collect_head_facts(repo)?;
    let worktree_facts = collect_worktree_facts(repo)?;

    let mut tracked = merge_tracked_facts(head_facts, index_facts, worktree_facts);

    if tracked.iter().any(|fact| needs_similarity_detection(fact)) {
        apply_similarity_detection(repo, &mut tracked)?;
    }

    Ok(tracked)
}
```

`apply_similarity_detection()` is the only place in this plan where narrow `git2` fallback is acceptable. Do not let `git2` spread into unrelated collection paths.

- [ ] **Step 4: Replace `parse_porcelain_v2()` in the production path with raw-record assembly**

After collection exists, switch the `Repository::status()` happy path to this shape.

```rust
let raw_records = collect_status_records(self, &plan, skip_untracked)?;
let mut entries: Vec<StatusEntry> = raw_records
    .into_iter()
    .map(assemble_status_entry)
    .collect();

if plan.needs_post_filter {
    post_filter_status_entries(&mut entries, &plan.combined_pathspecs);
}

Ok(entries)
```

- [ ] **Step 5: Keep `parse_porcelain_v2()` available only as a test/reference helper during the migration**

Move it behind a testing boundary or keep it private-but-unused only temporarily while parity tests still compare against the old semantics.

```rust
#[cfg(test)]
fn parse_porcelain_v2(data: &[u8]) -> Result<Vec<StatusEntry>, GitAiError> {
    // existing parser body retained only for migration-time parity checks
}
```

If a `#[cfg(test)]` move is too disruptive mid-task, keep the function private first and delete it in Task 5.

- [ ] **Step 6: Run targeted tests until the semantic contract is green**

Run:

```bash
task test TEST_FILTER=gix_status_index_comprehensive NO_CAPTURE=true
task test TEST_FILTER=e2big_post_filter NO_CAPTURE=true
```

Expected: PASS. The implementation is not acceptable if rename/orig_path, unmerged, or large-pathspec filtering regresses.

- [ ] **Step 7: Commit the in-process status collection switch**

```bash
git add src/git/status.rs tests/integration/gix_status_index_comprehensive.rs tests/integration/e2big_post_filter.rs
git commit -m "refactor: replace status porcelain cli with in-process collection"
```

---

## Task 4: Add direct parity checks against the old CLI semantics before removing the escape hatch

**Files:**
- Modify: `tests/integration/gix_status_index_comprehensive.rs`
- Modify: `src/git/status.rs` (only if needed to expose a test-only helper)

- [ ] **Step 1: Add a test-only helper that can still parse real porcelain v2 output for parity comparisons**

If `parse_porcelain_v2()` moved behind `#[cfg(test)]`, add a narrow helper inside `src/git/status.rs` tests or a test module.

```rust
#[cfg(test)]
pub(crate) fn parse_status_for_test(data: &[u8]) -> Result<Vec<StatusEntry>, GitAiError> {
    parse_porcelain_v2(data)
}
```

- [ ] **Step 2: Add a test helper that shells out to real `git status --porcelain=v2 -z` inside the integration test**

Add this helper to `tests/integration/gix_status_index_comprehensive.rs`.

```rust
fn cli_status_entries(repo: &TestRepo, args: &[&str]) -> Vec<StatusEntry> {
    let output = repo.git_og(args).expect("git status should succeed");
    git_ai::git::status::parse_status_for_test(output.as_bytes()).expect("porcelain should parse")
}
```

If the helper must use raw bytes instead of `String`, adapt it accordingly; the purpose is to compare the old source-of-truth parser against the new implementation for carefully chosen scenarios.

- [ ] **Step 3: Add one parity test for ordinary + untracked + rename behavior**

Add this test to `tests/integration/gix_status_index_comprehensive.rs`.

```rust
#[test]
fn gix_status_index_comprehensive_new_status_matches_cli_for_mixed_repo() {
    let repo = TestRepo::new();

    write_file(&repo, "rename-me.txt", "seed\n");
    write_file(&repo, "tracked.txt", "seed\n");
    repo.stage_all_and_commit("initial").unwrap();

    repo.git_og(&["mv", "rename-me.txt", "renamed.txt"]).unwrap();
    write_file(&repo, "tracked.txt", "seed\nunstaged\n");
    write_file(&repo, "untracked.txt", "brand new\n");

    let repository = open_repo(&repo);
    let mut new_entries = repository.status(None, false).unwrap();
    new_entries.sort_by(|left, right| left.path.cmp(&right.path));

    let mut cli_entries = cli_status_entries(
        &repo,
        &["status", "--porcelain=v2", "-z"],
    );
    cli_entries.sort_by(|left, right| left.path.cmp(&right.path));

    assert_eq!(new_entries, cli_entries);
}
```

- [ ] **Step 4: Run the parity scenario and focused regression suite**

Run:

```bash
task test TEST_FILTER=gix_status_index_comprehensive NO_CAPTURE=true
```

Expected: PASS. If this fails, fix parity before deleting any remaining old-path helpers.

- [ ] **Step 5: Commit the parity-check coverage**

```bash
git add src/git/status.rs tests/integration/gix_status_index_comprehensive.rs
git commit -m "test: compare in-process status with porcelain semantics"
```

---

## Task 5: Remove the remaining CLI-specific production scaffolding and run the final verification loop

**Files:**
- Modify: `src/git/status.rs`
- Test: `tests/integration/gix_status_index_comprehensive.rs`
- Test: `tests/integration/e2big_post_filter.rs`

- [ ] **Step 1: Delete the production-only CLI execution path from `Repository::status()`**

Remove the old `exec_git_with_profile(... status --porcelain=v2 -z ...)` path from `src/git/status.rs`. The production path should now only go through `collect_status_records()` and `assemble_status_entry()`.

```rust
// delete: run_status_cli(...)
// delete: production parse_porcelain_v2(...) usage
// keep: collect_status_records(...), post_filter_status_entries(...), assemble_status_entry(...)
```

- [ ] **Step 2: Delete or fully test-gate `parse_porcelain_v2()` once the parity suite is stable**

Pick one of these end states and complete it fully:

```rust
#[cfg(test)]
fn parse_porcelain_v2(data: &[u8]) -> Result<Vec<StatusEntry>, GitAiError> {
    // retained only for CLI parity tests
}
```

or:

```rust
// fully removed after parity tests are rewritten to use direct assertions only
```

Do not leave a half-used production helper behind.

- [ ] **Step 3: Run formatting and the focused verification suite**

Run:

```bash
task fmt
task test TEST_FILTER=gix_status_index_comprehensive
task test TEST_FILTER=e2big_post_filter
task test TEST_FILTER=status_ignore
```

Expected: PASS. `status_ignore` is included because this migration must not change higher-level status consumers indirectly.

- [ ] **Step 4: Run the full project verification required for this repo before handing off**

Run:

```bash
task lint
task test
```

Expected: PASS. If full `task test` reveals unrelated pre-existing failures, document them clearly before claiming completion.

- [ ] **Step 5: Commit the final cleanup**

```bash
git add src/git/status.rs tests/integration/gix_status_index_comprehensive.rs tests/integration/e2big_post_filter.rs
git commit -m "refactor: migrate status() off porcelain cli"
```

---

## Risk checklist during execution

- **Rename/copy drift:** If similarity detection starts leaking beyond one helper, stop and refactor it back behind a narrow boundary.
- **Post-filter drift:** Any regression where `orig_path` no longer matches a large pathspec is a release blocker.
- **Conflict handling drift:** The new implementation must not silently drop unresolved paths.
- **Scope creep:** Do not pull `repo_state.rs`, `commands/status.rs`, or unrelated repository helpers into this migration unless a failing test proves it is necessary.

## Verification checklist

- `Repository::status()` no longer shells out to `git status --porcelain=v2 -z`
- `StatusEntry` / `StatusCode` / `EntryKind` public API unchanged
- Large pathspec + non-ASCII pathspec behavior still matches old semantics
- `orig_path` filtering still works
- Unmerged entries are surfaced as `EntryKind::Unmerged`
- `task fmt`, `task lint`, and `task test` pass
