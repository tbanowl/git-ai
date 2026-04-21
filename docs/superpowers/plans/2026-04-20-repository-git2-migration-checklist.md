# `src/git/repository.rs` Git CLI → git2 Migration — Execution Checklist

**Date:** 2026-04-20
**Status:** Draft
**Source Requirements:** `docs/superpowers/specs/2026-04-20-repository-git2-migration-requirements.md`

## Phase 0 — Baseline Protection

- [ ] Review the requirements document and confirm scope remains limited to the
      first-phase migration targets.
- [ ] Add migration guard tests to lock current behavior before replacing CLI
      backends.
- [ ] Run the focused repository integration file as a baseline:

  ```bash
  cargo test --package git-ai --test git_repository_comprehensive -- --nocapture
  ```

## Phase 1 — Full Test Coverage in `git_repository_comprehensive.rs`

### Commit Metadata Tests

- [ ] Add root commit coverage.
- [ ] Add merge commit parent count and parent order coverage.
- [ ] Add summary/body parsing coverage.
- [ ] Add author/committer/time coverage.
- [ ] Add non-ASCII metadata coverage.

### Revparse and Reference Resolution Tests

- [ ] Add `HEAD`, branch name, full refname, and invalid revspec coverage.
- [ ] Add annotated tag and lightweight tag peel coverage.
- [ ] Add attached vs detached HEAD coverage.
- [ ] Add `find_reference()` and `references()` coverage.

### Commit Graph and Range Tests

- [ ] Add merge-base coverage.
- [ ] Add commit range length coverage.
- [ ] Add commit range iteration order coverage.
- [ ] Add commit range validity coverage.
- [ ] Add `parent_on_refname()` coverage.

### Object Access and Tree/Blob Content Tests

- [ ] Add object type coverage.
- [ ] Add `find_commit()` / `find_blob()` / `find_tree()` coverage.
- [ ] Add blob text and binary content coverage.
- [ ] Add `get_file_content()` root and nested path coverage.
- [ ] Add `Tree::get_path()` nested, non-ASCII, and spaced path coverage.

## Phase 2 — PR1: Commit Metadata Reads

- [ ] Replace CLI-backed `Commit::summary()`.
- [ ] Replace CLI-backed `Commit::body()`.
- [ ] Replace CLI-backed `Commit::author()`.
- [ ] Replace CLI-backed `Commit::committer()`.
- [ ] Replace CLI-backed `Commit::time()`.
- [ ] Replace CLI-backed `Commit::parents()`.
- [ ] Replace CLI-backed `Commit::parent_count()`.
- [ ] Replace CLI-backed `Commit::parent()`.
- [ ] Replace CLI-backed `Commit::tree()`.
- [ ] Run focused repository tests.
- [ ] Run full `cargo test`.

## Phase 3 — PR2: Revparse and Reference Resolution

- [ ] Replace CLI-backed `Repository::revparse_single()`.
- [ ] Replace CLI-backed `Object::peel_to_commit()`.
- [ ] Replace CLI-backed `Reference::shorthand()`.
- [ ] Replace CLI-backed `Reference::target()`.
- [ ] Replace CLI-backed `Reference::peel_to_blob()`.
- [ ] Replace CLI-backed `Reference::peel_to_commit()`.
- [ ] Replace CLI-backed `Repository::head()`.
- [ ] Replace CLI-backed `Repository::find_reference()`.
- [ ] Replace CLI-backed `Repository::references()`.
- [ ] Run focused repository tests.
- [ ] Run full `cargo test`.

## Phase 4 — PR3: Commit Graph and Range Queries

- [ ] Replace CLI-backed `Repository::merge_base()`.
- [ ] Replace CLI-backed `CommitRange::length()`.
- [ ] Replace CLI-backed `CommitRange::into_iter()`.
- [ ] Replace CLI-backed `CommitRange::is_valid()`.
- [ ] Replace CLI-backed `Commit::parent_on_refname()`.
- [ ] Verify commit range order remains unchanged.
- [ ] Verify ancestor semantics remain unchanged.
- [ ] Run focused repository tests.
- [ ] Run full `cargo test`.

## Phase 5 — PR4: Object Access and Content Reads

- [ ] Replace CLI-backed `Repository::object_type()`.
- [ ] Replace CLI-backed `Repository::find_commit()`.
- [ ] Replace CLI-backed `Repository::find_blob()`.
- [ ] Replace CLI-backed `Repository::find_tree()`.
- [ ] Replace CLI-backed `Blob::content()`.
- [ ] Replace CLI-backed `Repository::get_file_content()`.
- [ ] Replace CLI-backed `Tree::get_path()`.
- [ ] Run focused repository tests.
- [ ] Run full `cargo test`.

## Phase 6 — Final Verification

- [ ] Run the full test suite:

  ```bash
  cargo test
  ```

- [ ] Run linting:

  ```bash
  cargo clippy
  ```

- [ ] Run formatting check:

  ```bash
  cargo fmt -- --check
  ```

- [ ] Confirm no unintended changes were made to `diff_*` functions.
- [ ] Confirm no unintended changes were made to `merge_trees_favor_ours()`.
- [ ] Confirm no unintended changes were made to `fetch_branch()`.
- [ ] Confirm implementation still matches the requirements document.

## Critical Acceptance Checks

- [ ] Detached HEAD behavior is unchanged.
- [ ] Annotated tag peel behavior is unchanged.
- [ ] Merge commit parent order is unchanged.
- [ ] `CommitRange::into_iter()` ordering is unchanged.
- [ ] `%s` / `%b` commit message semantics are unchanged.
- [ ] Nested, non-ASCII, and spaced path behavior is unchanged.
