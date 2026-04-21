# `src/git/repository.rs` Git CLI → git2 Migration — Requirements

**Date:** 2026-04-20
**Status:** Draft

## Problem

`src/git/repository.rs` currently shells out to Git CLI for many local repository
read operations (`rev-parse`, `show`, `merge-base`, `rev-list`, `cat-file`,
`ls-tree`, etc.). In virtual machine environments, process startup and CLI I/O
overhead make these calls significantly slower than equivalent in-process Git
library access.

The goal is **not** to remove all CLI usage. The goal is to replace the
highest-frequency, lowest-risk, local read-only Git operations with `git2`
where it preserves existing behavior and measurably reduces VM overhead.

## Objective

Migrate the highest-value `repository.rs` Git CLI read paths to `git2` while
preserving the current public behavior, error semantics, and test coverage.

Success means:

- Reduced subprocess usage on hot local read paths.
- No intentional behavior drift in repository semantics.
- Existing and newly added integration tests pass unchanged.
- Diff/merge/network/write-heavy operations remain CLI-backed unless there is a
  strong reason to migrate later.

## Non-Goals

The following are **out of scope for the first phase**:

- Rewriting diff parsing logic.
- Replacing `merge-tree` behavior.
- Replacing `fetch` / other network-heavy operations.
- Large refactors of unrelated repository abstractions.
- Full Git CLI removal from `repository.rs`.
- Replacing existing `gix`-based index access just for consistency.

## Migration Principles

1. **Prefer high-frequency, local, read-only operations first.**
2. **Preserve existing repository contract before optimizing internals.**
3. **Do not mix semantic rewrites with performance migrations.**
4. **Keep CLI for operations whose current behavior depends on Git text output or
   porcelain semantics.**
5. **Add regression tests before or alongside migration.**

## Priority Tiers

### P0 — Must Migrate First

These are the best candidates because `git2` has strong API parity and they are
likely to be hit frequently in VM-sensitive flows.

- `Repository::revparse_single()`
- `Object::peel_to_commit()`
- `Commit::tree()`
- `Commit::parent()`
- `Commit::parents()`
- `Commit::parent_count()`
- `Commit::summary()`
- `Commit::body()`
- `Commit::author()`
- `Commit::committer()`
- `Commit::time()`
- `Repository::merge_base()`
- `CommitRange::length()`
- `CommitRange::into_iter()`
- `CommitRange::is_valid()`
- `Commit::parent_on_refname()`

### P1 — Recommended After P0

- `Reference::shorthand()`
- `Reference::target()`
- `Reference::peel_to_blob()`
- `Reference::peel_to_commit()`
- `Repository::head()`
- `Repository::find_reference()`
- `Repository::references()`
- `Repository::object_type()`
- `Repository::find_commit()`
- `Repository::find_blob()`
- `Repository::find_tree()`
- `Blob::content()`
- `Repository::get_file_content()`
- `Tree::get_path()`

### P2 — Optional / Later

- `remotes()` / `remotes_with_urls()`
- `new_infer_refname()`
- `find_repository()` discovery cleanup
- staged content reads that may be better served by existing `gix` primitives

### Keep CLI for Now

- `diff_*` functions
- `list_commit_files()`
- `merge_trees_favor_ours()`
- `commit()`
- `reference()`
- `fetch_branch()`

## PR Breakdown

### PR1 — Commit Metadata Reads

Scope:

- `Commit::summary()`
- `Commit::body()`
- `Commit::author()`
- `Commit::committer()`
- `Commit::time()`
- `Commit::parents()`
- `Commit::parent_count()`
- `Commit::parent()`
- `Commit::tree()`

### PR2 — Revparse and Reference Resolution

Scope:

- `Repository::revparse_single()`
- `Object::peel_to_commit()`
- `Reference::shorthand()`
- `Reference::target()`
- `Reference::peel_to_blob()`
- `Reference::peel_to_commit()`
- `Repository::head()`
- `Repository::find_reference()`
- `Repository::references()`

### PR3 — Commit Graph and Range Queries

Scope:

- `Repository::merge_base()`
- `CommitRange::length()`
- `CommitRange::into_iter()`
- `CommitRange::is_valid()`
- `Commit::parent_on_refname()`

### PR4 — Object Access and Content Reads

Scope:

- `Repository::object_type()`
- `Repository::find_commit()`
- `Repository::find_blob()`
- `Repository::find_tree()`
- `Blob::content()`
- `Repository::get_file_content()`
- `Tree::get_path()`

## Behavioral Requirements

All migrations must preserve the current externally observable behavior.

### Commit Metadata

- `summary()` must still behave like `%s`.
- `body()` must still behave like `%b`.
- `time()` must still reflect committer time semantics.
- root commits must report zero parents.
- merge commit parent order must remain stable.

### Reference and Revparse Semantics

- attached HEAD must still resolve to a symbolic branch ref.
- detached HEAD must still behave as current `Repository::head()` contract
  expects (returning `HEAD`, not surfacing a new error).
- annotated tag peeling must continue to resolve correctly to the underlying
  commit.
- invalid revspecs must still error.

### Commit Graph Semantics

- `CommitRange::into_iter()` order must remain identical to current behavior.
- `CommitRange::is_valid()` must preserve empty-tree special handling.
- `parent_on_refname()` must keep its current branch-name normalization and
  selection behavior.
- ancestor logic must preserve current behavior for `A == B`, root commits, and
  merge-heavy histories.

### Object/Path Semantics

- object type mismatch errors must remain explicit.
- `Blob::content()` must continue returning raw bytes.
- nested paths, non-ASCII paths, and paths with spaces must still work.
- missing paths must still error.

## Testing Requirements

All first-phase migration coverage should live in:

- `tests/integration/git_repository_comprehensive.rs`

with new sections appended rather than new test files initially.

### Required Test Sections

- `Commit Metadata Tests`
- `Revparse and Reference Resolution Tests`
- `Commit Graph and Range Tests`
- `Object Access and Tree/Blob Content Tests`

### Full-Coverage Test Matrix

#### Commit Metadata Tests

- `test_commit_summary_for_single_line_message`
- `test_commit_summary_and_body_for_multiline_message`
- `test_commit_body_is_empty_when_commit_has_no_body`
- `test_commit_author_and_committer_match_default_identity`
- `test_commit_author_and_committer_can_differ`
- `test_commit_time_uses_committer_time`
- `test_root_commit_has_no_parents`
- `test_commit_parent_zero_returns_first_parent`
- `test_commit_parent_out_of_bounds_errors`
- `test_merge_commit_parent_count_and_order_are_stable`
- `test_commit_tree_matches_head_tree_oid`
- `test_commit_metadata_supports_non_ascii_message_and_author`

#### Revparse and Reference Resolution Tests

- `test_revparse_single_resolves_head`
- `test_revparse_single_resolves_full_commit_oid`
- `test_revparse_single_resolves_branch_name`
- `test_revparse_single_resolves_fully_qualified_refname`
- `test_revparse_single_errors_for_invalid_spec`
- `test_object_peel_to_commit_from_commit_oid`
- `test_reference_peel_to_commit_from_annotated_tag`
- `test_reference_peel_to_commit_from_lightweight_tag`
- `test_reference_peel_to_blob_from_blob_spec`
- `test_reference_peel_to_commit_errors_for_non_commitish_reference`
- `test_reference_shorthand_matches_expected_branch_name`
- `test_reference_target_returns_expected_oid`
- `test_head_returns_symbolic_branch_ref_when_attached`
- `test_head_returns_head_when_detached`
- `test_find_reference_finds_existing_branch`
- `test_find_reference_finds_existing_tag`
- `test_find_reference_errors_for_missing_ref`
- `test_references_lists_heads_and_tags`
- `test_references_include_fully_qualified_refnames`

#### Commit Graph and Range Tests

- `test_merge_base_returns_common_ancestor_for_diverged_branches`
- `test_merge_base_errors_when_commits_are_invalid`
- `test_commit_range_length_for_linear_history`
- `test_commit_range_length_is_zero_for_adjacent_empty_range`
- `test_commit_range_into_iter_returns_expected_commits_in_current_order`
- `test_commit_range_into_iter_handles_single_commit_range`
- `test_commit_range_into_iter_returns_empty_for_empty_range`
- `test_commit_range_is_valid_when_start_is_ancestor_of_end`
- `test_commit_range_is_invalid_when_start_is_not_ancestor_of_end`
- `test_commit_range_is_invalid_when_start_is_not_reachable_from_refname`
- `test_commit_range_is_invalid_when_end_is_not_reachable_from_refname`
- `test_commit_range_allows_empty_tree_hash_as_start`
- `test_parent_on_refname_selects_parent_reachable_from_target_branch`
- `test_parent_on_refname_accepts_short_branch_name`
- `test_parent_on_refname_accepts_fully_qualified_refname`
- `test_parent_on_refname_errors_when_no_parent_is_reachable_from_ref`

#### Object Access and Tree/Blob Content Tests

- `test_object_type_reports_commit_blob_and_tree`
- `test_object_type_errors_for_missing_oid`
- `test_find_commit_returns_commit_for_commit_oid`
- `test_find_blob_returns_blob_for_blob_oid`
- `test_find_tree_returns_tree_for_tree_oid`
- `test_find_commit_errors_for_non_commit_oid`
- `test_find_blob_errors_for_non_blob_oid`
- `test_find_tree_errors_for_non_tree_oid`
- `test_blob_content_returns_exact_text_bytes`
- `test_blob_content_returns_exact_binary_bytes`
- `test_get_file_content_reads_file_from_commit_root`
- `test_get_file_content_reads_file_from_nested_path`
- `test_get_file_content_errors_for_missing_path`
- `test_get_file_content_errors_when_path_is_directory_like`
- `test_tree_get_path_returns_expected_entry_for_root_file`
- `test_tree_get_path_returns_expected_entry_for_nested_file`
- `test_tree_get_path_errors_for_missing_path`
- `test_get_file_content_supports_non_ascii_paths`
- `test_tree_get_path_supports_paths_with_spaces`

### Migration Guard Tests

These tests exist specifically to lock in current behavior before replacing CLI
backends:

- `test_detached_head_behavior_matches_current_repository_contract`
- `test_commit_range_iteration_order_matches_current_repository_contract`
- `test_merge_commit_parent_order_matches_current_repository_contract`
- `test_annotated_tag_peeling_matches_current_repository_contract`
- `test_summary_and_body_parsing_matches_current_repository_contract`
- `test_tree_path_lookup_matches_current_repository_contract_for_nested_paths`

## Verification Requirements

For each PR:

1. Run the focused integration test file:

   ```bash
   cargo test --package git-ai --test git_repository_comprehensive -- --nocapture
   ```

2. Run the full suite:

   ```bash
   cargo test
   ```

3. Before completion, also run:

   ```bash
   cargo clippy
   cargo fmt -- --check
   ```

## Risks

### Semantic Drift Risks

- `git rev-parse` may accept inputs or edge-case revspecs differently from
  `git2`.
- detached HEAD behavior may drift if `Repository::head()` is reinterpreted.
- merge-base / descendant APIs may differ on edge cases like `A == B`.
- summary/body parsing may drift if complete commit messages are handled
  differently from `%s` / `%b`.
- tree path lookup may drift for nested or unusual paths.

### Scope Risks

- Mixing diff/patch migration into this phase will dramatically increase risk.
- Folding in object writes (`commit-tree`, `update-ref`, `hash-object`) too early
  will complicate review and rollback.

## Acceptance Criteria

This requirements set is satisfied when:

- P0 migrations are implemented and reviewed in the proposed PR order.
- `git_repository_comprehensive.rs` contains full first-phase coverage.
- Migrated functions preserve existing repository semantics.
- No diff/merge/network operations were unintentionally folded into scope.
- The repository test suite, lint, and format checks all pass.
