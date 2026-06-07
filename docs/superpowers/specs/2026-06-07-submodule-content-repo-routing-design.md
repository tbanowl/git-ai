# Submodule Content Repository Routing Design

## Purpose

Make Git AI attribute file content to the repository that actually owns that content when a workspace contains Git submodules.

When an AI agent is launched from a parent repository but edits a file inside a submodule, checkpoints, working logs, and commit notes should be written for the submodule repository. The parent repository should only own `.gitmodules` and the gitlink pointer change for the submodule path.

## Problem

Claude Code and similar agents can run with the parent repository as their current working directory while editing files under a submodule path. Today the checkpoint path routing can keep those paths attached to the parent repository because:

- The Claude preset records `repo_working_dir` from the hook `cwd`, which is often the parent repository.
- The git-ai command handler only falls back to file-based repository detection when edited files appear to be outside the `cwd` repository workdir.
- `Repository::path_is_in_workdir()` currently treats submodule `.git` files as transparent to the parent workdir.
- `find_repository_for_file()` skips `.git` files that point into `.git/modules/...`, which prevents nearest-repository detection from selecting the submodule.

The result is that no submodule working log is created for parent-launched agent edits, and committing inside the submodule does not produce correct AI attribution in `refs/notes/ai`.

## Goals

- Treat Git submodules as independent repositories for all file content-level ownership decisions.
- Route checkpoints for `parent/submodule/file` to the submodule repository when the submodule is initialized and has its own `.git` boundary.
- Preserve existing behavior for ordinary files in the parent repository.
- Keep parent repository attribution focused on `.gitmodules` and gitlink pointer changes.
- Reuse existing file-based repository grouping instead of adding a separate submodule-only routing path.

## Non-Goals

- Do not copy submodule content attribution into parent repository commit notes.
- Do not rewrite or synchronize `refs/notes/ai` across parent and submodule histories.
- Do not invent attribution for uninitialized submodules that have no local repository metadata.
- Do not change the meaning of Git submodule pointer commits in the parent repository.
- Do not broaden this change to unrelated nested workspace conventions that are not represented by a Git repository boundary.

## Recommended Design

Use "nearest Git repository wins" as the content-path ownership rule.

For any path that refers to file contents, Git AI should resolve the closest repository boundary between the workspace root and the file. If the closest boundary is a submodule, the submodule owns the file for checkpoint, working-log, prompt metadata, blame, and note-generation purposes.

This keeps the main routing model simple:

1. Agent preset extracts edited file paths and the hook `cwd`.
2. Command handling normalizes paths to absolute file paths.
3. The current `cwd` repository is used only when those files truly belong to it.
4. If a path crosses an intervening Git repository boundary, file-based grouping routes it to the nearest repository.
5. Each repository receives its own checkpoint invocation and writes to its own `.git/ai` storage.

## Components

### Repository Boundary Detection

Update the repository discovery helpers in `src/git/repository.rs` so submodule `.git` files are repository boundaries for content paths.

- `has_intervening_git_dir()` should treat both `.git/` directories and `.git` files as boundaries.
- `.git` files should not be limited to linked worktree detection. A submodule `.git` file that points into the parent `.git/modules/...` directory still marks an independent repository.
- `Repository::path_is_in_workdir()` should return `false` for a content path that is physically under the parent workdir but crosses a submodule boundary.

This makes a submodule path look external to the parent repository for content ownership, which naturally triggers file-based repository routing.

### File Repository Discovery

Update `find_repository_for_file()` so it does not skip submodule `.git` files merely because they point inside `.git/modules/...`.

When a candidate directory contains a `.git` file, repository discovery should ask Git to open or discover the repository for that directory and let Git resolve the actual gitdir. For a submodule, the resolved repository workdir should be the submodule path, not the parent workdir.

The desired result is:

- `find_repository_for_file(parent/submodule/src/a.rs)` returns the submodule repository.
- `find_repository_for_file(parent/src/a.rs)` returns the parent repository.
- `find_repository_for_file(parent/.gitmodules)` returns the parent repository.

### Checkpoint Command Routing

The command handler in `src/commands/git_ai_handlers.rs` should continue to use the existing file-based grouping path.

Once `Repository::path_is_in_workdir()` treats submodule content paths as outside the parent content workdir, the existing `needs_file_based_repo_detection` branch should activate for parent-launched agent hooks that edit submodule files. `group_files_by_repository()` should then split files by repository and invoke checkpoint processing in the correct repo context.

No new submodule-specific command-line contract is needed.

## Data Flow

For a parent-launched Claude Code edit to `parent/libs/submod/src/lib.rs`:

1. Claude hook input includes the edited file path and `cwd = parent`.
2. The Claude preset returns `repo_working_dir = parent`.
3. Git AI normalizes the edited path to an absolute path.
4. Parent `path_is_in_workdir()` detects the intervening submodule `.git` file and returns `false` for content ownership.
5. File-based repository grouping calls repository discovery for the file.
6. Discovery opens the submodule repository at `parent/libs/submod`.
7. Checkpoint processing writes to the submodule's `.git/ai/working_logs/<base_commit>/`.
8. A later submodule `git commit` reads that submodule working log and writes the authorship log to the submodule's `refs/notes/ai`.

The parent repository may separately record a commit that updates `.gitmodules` or the submodule gitlink, but that parent note should not contain the submodule file's line-level content attribution.

## Edge Cases

- Uninitialized submodule path: if there is no `.git` file or `.git/` directory at the submodule path, the directory is not treated as an independent repository.
- Broken submodule gitdir: repository discovery should fail with the existing repository-open error path rather than silently attributing content to the parent.
- Nested submodules: nearest repository still wins, so the deepest initialized submodule owns its own content.
- Linked worktrees: existing linked worktree behavior should remain valid; `.git` files are boundaries for both linked worktrees and submodules.
- Ordinary nested repositories: existing independent nested repository behavior should continue to route to the nested repository.
- Parent `.gitmodules`: this file belongs to the parent repository because it is not inside the submodule repository boundary.
- Submodule gitlink path: the parent repository owns the gitlink entry as repository metadata, not the child content files.

## Compatibility

This is an intentional behavior change for true submodules. Any tests or comments that describe submodules as transparent to the parent repository for content attribution should be updated.

The new behavior aligns submodules with ordinary nested repositories for content-level attribution. It should not change attribution for files that do not cross a Git repository boundary.

## Testing

Use test-driven implementation. Add failing tests before changing repository discovery.

Required coverage:

- A real Git submodule under a parent repository where the agent hook `cwd` is the parent and the edited file is inside the submodule.
- The checkpoint is created in the submodule working log, not the parent working log.
- A submodule commit after that checkpoint produces an authorship note under the submodule's `refs/notes/ai`.
- The committed submodule file lines assert as AI using the existing committed-line attribution helpers.
- `find_repository_for_file()` returns the submodule repository for a file inside an initialized submodule.
- Parent `Repository::path_is_in_workdir()` returns `false` for submodule content paths.
- Existing nested independent repository tests continue to pass.
- Existing tests that expected submodule transparency are updated to assert independent submodule ownership.

Tests that care about exact checkpoint flow should use explicit file writes plus `human`, `mock_ai`, or `mock_known_human` checkpoints rather than the high-level `set_contents` helper.

Focused verification should start with the new submodule routing tests, then run the nearby integration tests:

- `task test TEST_FILTER=multi_repo_workspace`
- `task test TEST_FILTER=cross_repo_cwd_attribution`
- `task test TEST_FILTER=git_repository_comprehensive`

Before completion, run the project-required checks for the final change set:

- `task build`
- `task fmt`
- `task lint`

## Risks

The main risk is changing a low-level repository ownership predicate that other flows rely on. Keeping the API surface mostly unchanged and routing through the existing file-based grouping path reduces that risk.

Another risk is confusing content ownership with parent repository metadata ownership. The implementation should keep the distinction explicit: submodule files belong to the submodule; `.gitmodules` and gitlink pointer changes belong to the parent.

Broken submodule metadata can also expose hard-to-interpret errors. The first implementation should prefer visible repository discovery errors over silently recording attribution in the wrong repository.

## Acceptance Criteria

- Parent-launched AI edits inside an initialized submodule generate checkpoints in the submodule repository.
- Submodule commits after those checkpoints include AI attribution notes in the submodule's `refs/notes/ai`.
- Parent repository commits do not claim line-level attribution for submodule file contents.
- `.gitmodules` and gitlink pointer updates remain parent repository changes.
- Existing non-submodule repository discovery behavior remains covered by tests.
