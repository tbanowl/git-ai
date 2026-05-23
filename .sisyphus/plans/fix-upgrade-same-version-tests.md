# Fix Upgrade Same-Version Tests

## TL;DR
> **Summary**: Fix the upgrade action decision so release semver and current package version are normalized consistently, resolving the three failing `commands::upgrade` tests where same-version cases incorrectly return `UpgradeAvailable`.
> **Deliverables**:
> - Focused regression test for current versions with packaging suffixes such as `1.3.4-1`
> - Minimal fix in `src/commands/upgrade.rs::determine_action()`
> - Targeted verification for the three previously failing tests and the upgrade test module
> **Effort**: Quick
> **Parallel**: NO
> **Critical Path**: Add failing regression test → Normalize current version in `determine_action()` → Verify targeted upgrade tests

## Context

### Original Request
User asked: “帮我修复 3、4、5 的测试” after full test run reported failures in:

1. `commands::upgrade::tests::test_check_for_update_available_same_version`
2. `commands::upgrade::tests::test_run_impl_with_url`
3. `commands::upgrade::tests::test_run_impl_with_url_enterprise_channels`

The user only asked to fix these three upgrade tests. Snapshot failures in `authorship::stats` are out of scope.

### Interview Summary
- Scope is explicit from prior test output: fix failing tests 3/4/5 only.
- No user preference question is required because the failing assertions already define the intended behavior: same/current version cases must return `AlreadyLatest`.
- Project canonical test command is `task test`, but this environment lacks the `task` binary. `Taskfile.yml` shows the equivalent full-test command is cargo test with `GIT_AI_TEST_GIT_MODE=daemon` and shared daemon pool env vars.

### Research Findings
- `Cargo.toml:3` sets package version to `1.3.4-1`; `env!("CARGO_PKG_VERSION")` therefore yields `"1.3.4-1"`.
- `src/commands/upgrade.rs:282-288` defines `semver_from_tag()`, which strips `enterprise-`, leading `v`, and suffix after `-` or `+`; `semver_from_tag("v1.3.4-1") == "1.3.4"`.
- `src/commands/upgrade.rs:290-302` defines `determine_action()`, which currently compares normalized `release.semver` to raw `current_version`.
- `src/commands/upgrade.rs:1017-1036` defines `is_newer_version()` and `parse_version()`; `parse_version("1.3.4-1")` drops the `"4-1"` component and effectively compares current as `[1, 3]`.
- Failing assertions are at `src/commands/upgrade.rs:1148`, `src/commands/upgrade.rs:1219`, and `src/commands/upgrade.rs:1763`.

### Metis Review (gaps addressed)
- Keep the fix narrow: do not redesign semver handling.
- Normalize `current_version` inside `determine_action()` so every caller benefits consistently.
- Add an explicit regression test with string literals rather than relying only on `env!("CARGO_PKG_VERSION")`.
- Do not change `semver_from_tag()`, `is_newer_version()`, `Cargo.toml`, HTTP mocks, install behavior, channel selection, or snapshot tests.

## Work Objectives

### Core Objective
Make upgrade same-version checks return `UpgradeAction::AlreadyLatest` when the current package version has a packaging suffix (`1.3.4-1`) and the normalized release semver is the same base version (`1.3.4`).

### Deliverables
- One focused regression test in `src/commands/upgrade.rs`.
- One minimal production code change in `src/commands/upgrade.rs::determine_action()`.
- Verification evidence showing the new regression test, the three previously failing tests, and all upgrade tests pass.

### Definition of Done (verifiable conditions with commands)
- `cargo test --lib commands::upgrade::tests::test_determine_action_same_version_with_current_suffix` exits 0.
- `cargo test --lib commands::upgrade::tests::test_run_impl_with_url` exits 0.
- `cargo test --lib commands::upgrade::tests::test_run_impl_with_url_enterprise_channels` exits 0.
- `cargo test --lib commands::upgrade::tests::test_check_for_update_available_same_version` exits 0.
- `cargo test --lib commands::upgrade::tests` exits 0.
- `cargo fmt --check` exits 0.
- If `task` is available to the worker, `task lint` exits 0. If `task` is unavailable, record that fact in evidence and do not install tooling unless explicitly requested.

### Must Have
- Preserve behavior of `force == true`: always return `UpgradeAction::ForceReinstall` before version normalization.
- Preserve behavior of clean same versions: `release.semver = "1.3.4"`, `current_version = "1.3.4"` → `AlreadyLatest`.
- Preserve behavior of newer releases: `release.semver = "999.0.0"`, `current_version = "1.3.4-1"` → `UpgradeAvailable`.
- Preserve behavior of older releases: `release.semver = "1.0.9"`, `current_version = "1.3.4-1"` → `RunningNewerVersion`.

### Must NOT Have
- Do not fix or update snapshot failures in `authorship::stats`.
- Do not introduce a `semver` crate dependency.
- Do not rewrite `is_newer_version()` or `parse_version()` in this task.
- Do not change `semver_from_tag()`.
- Do not change `Cargo.toml` package version.
- Do not refactor upgrade networking, release fetching, installer behavior, channel selection, or CLI output.
- Do not run snapshot review/accept commands.

## Verification Strategy
> ZERO HUMAN INTERVENTION - all verification is agent-executed.
- Test decision: TDD using Rust unit tests in the existing `cargo test` framework.
- QA policy: Every task has agent-executed scenarios.
- Evidence: `.sisyphus/evidence/task-{N}-{slug}.{ext}`

## Execution Strategy

### Parallel Execution Waves
> Target: 5-8 tasks per wave. <3 per wave (except final) = acceptable here because this is a narrow single-file bug fix with sequential TDD dependency.
> Extract shared dependencies as Wave-1 tasks for max parallelism.

Wave 1: Task 1 only — red/green regression and minimal fix in `src/commands/upgrade.rs`.

### Dependency Matrix (full, all tasks)
- Task 1 has no implementation dependencies.
- Final Verification Wave depends on Task 1.

### Agent Dispatch Summary (wave → task count → categories)
- Wave 1 → 1 task → `quick`
- Final Verification Wave → 4 review agents → `oracle`, `unspecified-high`, `unspecified-high`, `deep`

## TODOs
> Implementation + Test = ONE task. Never separate.
> EVERY task MUST have: Agent Profile + Parallelization + QA Scenarios.

- [ ] 1. Normalize current upgrade version comparison

  **What to do**:
  1. Open `src/commands/upgrade.rs`.
  2. Find the existing `determine_action(force: bool, release: &ChannelRelease, current_version: &str) -> UpgradeAction` near `src/commands/upgrade.rs:290-302`.
  3. Before modifying production code, add a focused regression test in the existing `#[cfg(test)] mod tests` section near the existing `determine_action` tests. Use this exact test name and body:

     ```rust
     #[test]
     fn test_determine_action_same_version_with_current_suffix() {
         let release = ChannelRelease {
             tag: "v1.3.4-1".to_string(),
             semver: "1.3.4".to_string(),
             checksum: "test-checksum".to_string(),
         };

         assert_eq!(
             determine_action(false, &release, "1.3.4-1"),
             UpgradeAction::AlreadyLatest
         );
     }
     ```

  4. Run the new regression test before implementation:

     ```bash
     cargo test --lib commands::upgrade::tests::test_determine_action_same_version_with_current_suffix
     ```

     Expected before implementation: command exits non-zero and the assertion reports `UpgradeAvailable` instead of `AlreadyLatest`.

  5. Modify only `determine_action()` so `current_version` is normalized with the existing canonical normalization before equality and newer-version comparison. Use this exact implementation shape:

     ```rust
     fn determine_action(force: bool, release: &ChannelRelease, current_version: &str) -> UpgradeAction {
         if force {
             return UpgradeAction::ForceReinstall;
         }

         let current_semver = semver_from_tag(current_version);

         if release.semver == current_semver {
             UpgradeAction::AlreadyLatest
         } else if is_newer_version(&release.semver, &current_semver) {
             UpgradeAction::UpgradeAvailable
         } else {
             UpgradeAction::RunningNewerVersion
         }
     }
     ```

  6. Do not alter callers. Existing callers should continue passing raw `env!("CARGO_PKG_VERSION")`; `determine_action()` is the normalization boundary.
  7. Do not alter `semver_from_tag()`, `is_newer_version()`, or `parse_version()`.
  8. Run targeted verification commands listed below and save outputs to evidence files.

  **Must NOT do**:
  - Do not change tests 1/2 snapshot files.
  - Do not run `cargo insta review` or `cargo insta accept`.
  - Do not add dependencies.
  - Do not change `Cargo.toml`.
  - Do not broadly refactor upgrade code.

  **Recommended Agent Profile**:
  - Category: `quick` - Reason: single-file, narrow Rust unit-test bug fix with clear root cause and acceptance criteria.
  - Skills: [`superpowers:test-driven-development`, `superpowers:systematic-debugging`] - TDD is required for the focused regression; systematic debugging prevents over-broad semver rewrites.
  - Omitted: [`frontend-patterns`, `api-design`] - No frontend or API design changes are involved.

  **Parallelization**: Can Parallel: NO | Wave 1 | Blocks: Final Verification Wave | Blocked By: none

  **References** (executor has NO interview context - be exhaustive):
  - Pattern: `src/commands/upgrade.rs:282-288` - `semver_from_tag()` is the existing canonical normalizer for release tags; reuse it for current version normalization.
  - Pattern: `src/commands/upgrade.rs:290-302` - `determine_action()` is the minimal comparison boundary to change.
  - Pattern: `src/commands/upgrade.rs:1017-1036` - `is_newer_version()` currently receives raw current versions and misinterprets suffixes; do not rewrite it, just pass normalized inputs.
  - Test: `src/commands/upgrade.rs:1148` - failing assertion in `test_run_impl_with_url` expects `AlreadyLatest`.
  - Test: `src/commands/upgrade.rs:1219` - failing assertion in `test_run_impl_with_url_enterprise_channels` expects `AlreadyLatest`.
  - Test: `src/commands/upgrade.rs:1763` - failing assertion in `test_check_for_update_available_same_version` expects `AlreadyLatest`.
  - Config: `Cargo.toml:3` - current package version is `1.3.4-1`, which triggers the bug.

  **Acceptance Criteria** (agent-executable only):
  - [ ] New regression test fails before production code change with `UpgradeAvailable` vs `AlreadyLatest`.
  - [ ] `cargo test --lib commands::upgrade::tests::test_determine_action_same_version_with_current_suffix` exits 0 after the fix.
  - [ ] `cargo test --lib commands::upgrade::tests::test_run_impl_with_url` exits 0 after the fix.
  - [ ] `cargo test --lib commands::upgrade::tests::test_run_impl_with_url_enterprise_channels` exits 0 after the fix.
  - [ ] `cargo test --lib commands::upgrade::tests::test_check_for_update_available_same_version` exits 0 after the fix.
  - [ ] `cargo test --lib commands::upgrade::tests` exits 0 after the fix.
  - [ ] `cargo fmt --check` exits 0.
  - [ ] Evidence files exist for red failure, green focused tests, upgrade module tests, and formatting check.

  **QA Scenarios** (MANDATORY - task incomplete without these):
  ```
  Scenario: Current version with packaging suffix is treated as already latest
    Tool: Bash
    Steps:
      1. Run: cargo test --lib commands::upgrade::tests::test_determine_action_same_version_with_current_suffix
      2. Capture stdout/stderr to .sisyphus/evidence/task-1-current-suffix-regression.txt
    Expected: Command exits 0; output includes `test_determine_action_same_version_with_current_suffix ... ok` or `test result: ok`.
    Evidence: .sisyphus/evidence/task-1-current-suffix-regression.txt

  Scenario: Previously failing same-version upgrade tests pass
    Tool: Bash
    Steps:
      1. Run: cargo test --lib commands::upgrade::tests::test_run_impl_with_url
      2. Run: cargo test --lib commands::upgrade::tests::test_run_impl_with_url_enterprise_channels
      3. Run: cargo test --lib commands::upgrade::tests::test_check_for_update_available_same_version
      4. Capture all outputs to .sisyphus/evidence/task-1-existing-failing-tests.txt
    Expected: Each command exits 0; no assertion shows `UpgradeAvailable` where `AlreadyLatest` is expected.
    Evidence: .sisyphus/evidence/task-1-existing-failing-tests.txt

  Scenario: Broader upgrade module remains green
    Tool: Bash
    Steps:
      1. Run: cargo test --lib commands::upgrade::tests
      2. Capture output to .sisyphus/evidence/task-1-upgrade-module.txt
    Expected: Command exits 0; output ends with `test result: ok`.
    Evidence: .sisyphus/evidence/task-1-upgrade-module.txt

  Scenario: Formatting remains valid
    Tool: Bash
    Steps:
      1. Run: cargo fmt --check
      2. Capture output to .sisyphus/evidence/task-1-format-check.txt
    Expected: Command exits 0; rustfmt reports no changes required.
    Evidence: .sisyphus/evidence/task-1-format-check.txt
  ```

  **Commit**: YES | Message: `fix(upgrade): normalize current version comparison` | Files: [`src/commands/upgrade.rs`]

## Final Verification Wave (MANDATORY — after ALL implementation tasks)
> 4 review agents run in PARALLEL. ALL must APPROVE. Present consolidated results to user and get explicit "okay" before completing.
> **Do NOT auto-proceed after verification. Wait for user's explicit approval before marking work complete.**
> **Never mark F1-F4 as checked before getting user's okay.** Rejection or user feedback -> fix -> re-run -> present again -> wait for okay.
- [ ] F1. Plan Compliance Audit — oracle
- [ ] F2. Code Quality Review — unspecified-high
- [ ] F3. Real Manual QA — unspecified-high
- [ ] F4. Scope Fidelity Check — deep

## Commit Strategy
- Make exactly one implementation commit after all Task 1 acceptance criteria pass.
- Commit message: `fix(upgrade): normalize current version comparison`.
- Stage only `src/commands/upgrade.rs` unless evidence files are intentionally tracked by the workflow; do not stage snapshot files, `Cargo.toml`, or unrelated formatting changes.
- If a git hook changes formatting, inspect the resulting diff and rerun targeted verification before committing.

## Success Criteria
- The three requested failing tests pass.
- The focused regression test demonstrates the intended suffix-normalization behavior.
- All upgrade tests pass.
- No out-of-scope snapshot files or dependency files are changed.
- Final verification agents approve and user explicitly confirms completion.
