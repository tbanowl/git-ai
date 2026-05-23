# `resolve_git_var_identity()` CLI → in-process migration design

**Date:** 2026-04-26
**Status:** Draft

## 1. Goal

Migrate `src/git/repository.rs::resolve_git_var_identity()` off the `git var`
subprocess path and onto an in-process implementation, while preserving the
behavior that matters to current callers and tests.

This is intentionally a **single-function** migration. Even though
`repository.rs` still contains other CLI-backed operations, this design only
covers the one remaining low-difficulty item that can be migrated without
changing semantics in adjacent code.

## 2. Current behavior

Today `resolve_git_var_identity()` shells out to Git:

```text
git var GIT_COMMITTER_IDENT
git var GIT_AUTHOR_IDENT
```

It then parses the output with `parse_git_var_identity()`. If the CLI call
fails, it falls back to reading `user.name` and `user.email` through the
existing `config_get_str()` path.

That means the current effective behavior is:

1. Prefer Git's resolved identity when `git var` succeeds.
2. Fall back to repo/global config reads if `git var` fails.
3. Return a `GitAuthorIdentity` with the existing parsing/formatting behavior.

This function feeds two public entry points:

- `Repository::git_author_identity()`
- `Repository::git_commit_author_identity()`

## 3. Scope

### In scope

- Replace the `exec_git(["var", ...])` path inside
  `resolve_git_var_identity()`.
- Preserve the current `GitAuthorIdentity` return type and caller contracts.
- Preserve the behavior already covered by integration tests:
  - identity shape remains stable
  - environment variables override config values
- Add or adjust tests as needed to lock the new behavior in place.

### Out of scope

- Migrating `git_version()`.
- Migrating any medium/high-difficulty CLI usage in `repository.rs`.
- Refactoring unrelated repository helpers.
- Replacing `parse_git_var_identity()` with a new API.
- Perfectly reproducing every obscure `git var` internal identity derivation
  rule if it is not currently relied on by this codebase.

## 4. Why only this function

During scoping, the remaining `repository.rs` CLI calls split into three groups:

1. **Low difficulty and semantically safe to migrate now**
   - `resolve_git_var_identity()`
2. **Low difficulty on paper but semantically risky**
   - `git_version()` because it currently reflects **system Git version**,
     whereas a library replacement would expose **libgit2 version**
3. **Medium or higher difficulty**
   - `blob()`
   - `reference()`
   - `commit_range_on_branch()`
   - `resolve_author_spec()`
   - `commit()`
   - etc.

This makes `resolve_git_var_identity()` the only migration candidate that is
both low-risk and still clearly within the requested “low difficulty” scope.

## 5. Proposed implementation

### 5.1 Resolution order

Replace the CLI-first implementation with an in-process identity resolver that
uses the same high-level precedence expected by the current tests:

1. Check environment variables for the requested identity kind
2. Fall back to existing config lookup helpers
3. Return default/partial identity if values are absent

For `GIT_COMMITTER_IDENT`, prefer:

- `GIT_COMMITTER_NAME`
- `GIT_COMMITTER_EMAIL`

For `GIT_AUTHOR_IDENT`, prefer:

- `GIT_AUTHOR_NAME`
- `GIT_AUTHOR_EMAIL`

If the requested variable is unknown, return the same empty/default style
result used by the current fallback behavior rather than inventing new errors.

### 5.2 Config fallback

Retain the existing config fallback logic already present in the function:

- `self.config_get_str("user.name")`
- `self.config_get_str("user.email")`

This keeps the migration aligned with current repository-level behavior and
reuses existing `gix-config` backed code instead of introducing a new config
stack for this one function.

### 5.3 Parsing behavior

`parse_git_var_identity()` stays in place.

However, after this migration the main path will often construct
`GitAuthorIdentity` directly from environment/config data instead of round-
tripping through a synthesized `git var` output string. That avoids fake CLI
format reconstruction and keeps the implementation simpler.

`parse_git_var_identity()` should remain tested because it is still useful and
encodes formatting assumptions already present in the file.

## 6. Behavioral contract

The migration must preserve these externally observable behaviors:

### 6.1 Must preserve

- `git_author_identity()` remains cached through the existing `OnceLock`.
- `git_commit_author_identity()` remains uncached.
- Environment variables override config values.
- Missing name or email still produces partial identities in the current style.
- Formatting helpers continue to behave the same.

### 6.2 Allowed change

It is acceptable that we no longer depend on the exact `git var` subprocess
success/failure behavior, as long as the resulting identity resolution observed
by callers remains equivalent for supported cases.

### 6.3 Explicit non-goal

This migration does **not** promise byte-for-byte parity with every possible
`git var` edge case, especially if Git would synthesize values beyond env/config
inputs. That is a conscious scope boundary for keeping this change low-risk.

## 7. Testing strategy

This change should follow TDD and reuse existing repository-level test patterns.

### 7.1 Existing tests to retain/update

Primary existing coverage:

- `tests/integration/low_difficulty_git2_gix_task1.rs`
  - validates identity parsing/shape
  - validates env-over-config precedence

These tests should continue passing without depending on `exec_git("var")`.

### 7.2 New tests to add

Add focused tests for pure parsing/formatting behavior in
`src/git/repository.rs` unit tests:

- `parse_git_var_identity()` parses `Name <email> timestamp tz`
- parser handles name-only input
- parser handles empty input

If coverage is still light after implementation, add one integration case for
author identity resolution specifically through `git_commit_author_identity()` so
both author and committer env branches are exercised.

## 8. Implementation options considered

### Option A — Direct env + config resolution (recommended)

Read `GIT_AUTHOR_*` / `GIT_COMMITTER_*` directly, then fall back to
`config_get_str()`.

**Pros**

- Smallest diff
- Preserves tested precedence
- No subprocess
- Reuses existing config plumbing

**Cons**

- Does not emulate every hidden `git var` rule

### Option B — Use git2 config APIs directly

Replace the config fallback with `git2::Repository::config()` and derive the
identity from there.

**Pros**

- More “pure git2” on paper

**Cons**

- Adds a second config access path beside the existing `gix-config` helpers
- Increases change surface for no clear behavioral win
- Still does not solve env precedence by itself without extra logic

### Option C — Reconstruct a synthetic `git var` string then parse it

Build `"Name <email> ..."` manually and keep using `parse_git_var_identity()`.

**Pros**

- Reuses parser on the hot path

**Cons**

- Artificial and unnecessary
- Requires inventing timestamp/timezone content we do not actually need

### Recommendation

Choose **Option A**.

It is the smallest and clearest change that preserves the behavior we care
about, stays within the requested scope, and avoids introducing a second new
abstraction layer.

## 9. Files expected to change

- `src/git/repository.rs`
- `tests/integration/low_difficulty_git2_gix_task1.rs`
- possibly `src/git/repository.rs` unit test section for parser coverage

## 10. Risks and mitigations

### Risk: author vs committer env precedence diverges

**Mitigation:** add explicit tests for both `GIT_AUTHOR_*` and
`GIT_COMMITTER_*` paths.

### Risk: config fallback behavior drifts

**Mitigation:** keep using `config_get_str()` instead of replacing that stack.

### Risk: over-scoping into adjacent migrations

**Mitigation:** treat any change outside `resolve_git_var_identity()` as out of
scope unless a test forces a minimal supporting adjustment.

## 11. Approval boundary

Once this design is approved, the next step is to write a concrete implementation
plan for the single-function migration and then execute it with TDD.
