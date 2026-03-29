# REST Notes Store Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a configurable REST-backed notes sync mode (`notes_store=rest`) so git-ai can sync authorship notes when remotes do not support `refs/notes/ai`.

**Architecture:** Keep local git-notes read/write unchanged (`src/git/refs.rs`). Branch only in `src/git/sync_authorship.rs`: current git path for `notes_store=git`, REST API path for `notes_store=rest`. Reuse existing `ApiClient`/`ApiContext` conventions and existing remote resolution patterns.

**Tech Stack:** Rust 2024, minreq via existing API client abstractions, serde/serde_json, git CLI wrappers, Flask + SQLAlchemy (server-side in separate repo).

---

## Guardrails

- Do not change local notes primitives (`note_blob_oids_for_commits`, `notes_add_batch`, `show_authorship_note`) except tests.
- Do not change hook dispatch wiring; only sync behavior behind existing functions.
- Preserve current `NotesExistence` semantics (`NotFound` means remote has no notes, not generic HTTP failures).
- Use repo URL normalization before REST calls.

---

### Task 1: Add notes_store config surface

**Files:**
- Modify: `src/config.rs`
- Modify: `src/commands/config.rs`
- Test: `src/config.rs` tests

**Step 1: Write failing tests**

```rust
#[test]
fn test_notes_store_default_is_git() {
    let cfg = create_test_config(vec![], vec![]);
    assert_eq!(cfg.notes_store(), "git");
}
```

Add tests for invalid value fallback and test patch override.

**Step 2: Run tests (fail first)**

Run: `cargo test --package git-ai --lib test_notes_store_default_is_git -- --nocapture`
Expected: FAIL before implementation.

**Step 3: Minimal implementation**

- Add `notes_store: String` to `Config`
- Add `notes_store: Option<String>` to `FileConfig`
- Add `notes_store: Option<String>` to `ConfigPatch`
- In `build_config()`, implement precedence: `GIT_AI_NOTES_STORE` > file config > default `"git"`
- Validate only `git|rest`, warn/fallback on invalid value
- Add getter: `pub fn notes_store(&self) -> &str`
- In `apply_test_config_patch`, support patching `notes_store`
- In `src/commands/config.rs`, expose `notes_store` in help/show/get/set/unset flows

**Step 4: Re-run tests**

Run: `cargo test --package git-ai --lib test_notes_store_default_is_git -- --nocapture`
Expected: PASS.

**Step 5: Commit**

```bash
git add src/config.rs src/commands/config.rs
git commit -m "feat(config): add notes_store backend selector"
```

---

### Task 2: Add REST notes API payload types

**Files:**
- Modify: `src/api/types.rs`
- Test: `src/api/types.rs` tests

**Step 1: Write failing tests**

```rust
#[test]
fn test_notes_list_response_has_blob_identity() {
    let body = r#"{"ok":true,"data":{"notes":[{"commit_sha":"abc","note_blob_oid":"deadbeef"}]}}"#;
    let parsed: NotesListResponse = serde_json::from_str(body).unwrap();
    assert_eq!(parsed.data.notes[0].commit_sha, "abc");
}
```

**Step 2: Run tests (fail first)**

Run: `cargo test --package git-ai --lib test_notes_list_response_has_blob_identity -- --nocapture`
Expected: FAIL.

**Step 3: Minimal implementation**

- Add request/response structs for list/batch/push
- Include `commit_sha` + `note_blob_oid` (or equivalent stable note hash) in list/batch success payloads
- Reuse existing `ApiErrorResponse` for non-200 handling path

**Step 4: Re-run tests**

Run: `cargo test --package git-ai --lib test_notes_list_response_has_blob_identity -- --nocapture`
Expected: PASS.

**Step 5: Commit**

```bash
git add src/api/types.rs
git commit -m "feat(api): add notes REST request and response types"
```

---

### Task 3: Implement notes API client module

**Files:**
- Create: `src/api/notes_api.rs`
- Modify: `src/api/mod.rs`
- Test: `src/api/notes_api.rs` tests

**Step 1: Write failing tests**

Add tests for success parse and non-200 error parse fallback.

**Step 2: Run tests (fail first)**

Run: `cargo test --package git-ai --lib notes_api -- --nocapture`
Expected: FAIL.

**Step 3: Minimal implementation**

- Add `impl ApiClient` methods for:
  - `notes_list`
  - `notes_batch_get`
  - `notes_push`
- Use existing style: `self.context().post_json(...)`, status match, body parse, `ApiErrorResponse` fallback
- Use client construction pattern: `ApiClient::new(ApiContext::new(None))`

**Step 4: Re-run tests**

Run: `cargo test --package git-ai --lib notes_api -- --nocapture`
Expected: PASS.

**Step 5: Commit**

```bash
git add src/api/notes_api.rs src/api/mod.rs src/api/types.rs
git commit -m "feat(api): implement notes REST client methods"
```

---

### Task 4: Pre-branch regression safety net (must run before REST branching)

**Files:**
- Test: `tests/integration/internal_machine_commands.rs`
- Test: `tests/integration/notes_sync_regression.rs`

**Step 1: Add failing regression assertions**

- Default mode (`notes_store` unset) remains current git behavior
- Existing internal machine commands retain expected JSON semantics

**Step 2: Run targeted tests**

Run: `cargo test --package git-ai --test integration test_fetch_and_push_authorship_notes_internal_commands_json -- --nocapture`
Run: `cargo test --package git-ai --test notes_sync_regression -- --nocapture`
Expected: PASS after assertions are aligned; fix test assumptions if needed.

**Step 3: Commit**

```bash
git add tests/integration/internal_machine_commands.rs tests/integration/notes_sync_regression.rs
git commit -m "test(sync): lock existing git-notes behavior before REST mode"
```

---

### Task 5: Implement REST branch in sync_authorship

**Files:**
- Modify: `src/git/sync_authorship.rs`
- Modify: `src/api/notes_api.rs` (if helper methods needed)

**Step 1: Write failing integration tests**

Create failing tests for:
- empty remote list => `NotesExistence::NotFound`
- remote note exists => fetch returns `Found`
- changed remote/local note identity causes update write

**Step 2: Minimal implementation**

- In `fetch_authorship_notes` and `push_authorship_notes`, branch on `Config::get().notes_store() == "rest"`
- Resolve remote URL from existing remote-name-or-url flows
- Normalize URL via `crate::repo_url::normalize_repo_url` before REST calls
- `rest_fetch_notes`:
  1. call list
  2. if empty -> `NotFound`
  3. compare remote/local by `commit_sha + note_blob_oid`
  4. fetch missing/changed content
  5. write through `notes_add_batch`
  6. return `Found`
- `rest_push_notes`:
  1. enumerate local notes (`git notes --ref=ai list` pattern)
  2. compare with remote note identities
  3. push changed/missing notes only

**Step 3: Run targeted tests**

Run: `cargo test --package git-ai --test integration rest_notes_sync -- --nocapture`
Expected: PASS.

**Step 4: Commit**

```bash
git add src/git/sync_authorship.rs src/api/notes_api.rs
git commit -m "feat(sync): add REST notes fetch/push backend path"
```

---

### Task 6: Add REST integration test scaffolding and error contracts

**Files:**
- Create: `tests/integration/rest_test_server.rs`
- Create: `tests/integration/rest_notes_sync.rs`
- Create: `tests/integration/rest_notes_errors.rs`
- Modify: `tests/integration/main.rs`

**Step 1: Write failing tests**

- 200 empty list => `NotFound`
- HTTP 404 from endpoint => hard error
- auth/network failures => hard error

**Step 2: Implement test helper/server**

- Add stdlib `TcpListener` based lightweight mock server
- Route responses per test case
- Ensure tests set `GIT_AI_API_BASE_URL` to mock server

**Step 3: Run focused integration tests**

Run: `cargo test --package git-ai --test integration rest_notes_sync -- --nocapture`
Run: `cargo test --package git-ai --test integration rest_notes_errors -- --nocapture`
Expected: PASS.

**Step 4: Commit**

```bash
git add tests/integration/rest_test_server.rs tests/integration/rest_notes_sync.rs tests/integration/rest_notes_errors.rs tests/integration/main.rs
git commit -m "test(sync): add REST notes integration and error-contract tests"
```

---

### Task 7: Full verification gate

**Step 1: Focused tests**

Run:
- `cargo test --package git-ai --test integration test_fetch_and_push_authorship_notes_internal_commands_json -- --nocapture`
- `cargo test --package git-ai --test integration rest_notes_sync -- --nocapture`
- `cargo test --package git-ai --test integration rest_notes_errors -- --nocapture`
- `cargo test --package git-ai --test notes_sync_regression -- --nocapture`

**Step 2: Quality checks**

Run:
- `cargo fmt -- --check`
- `cargo clippy`
- `cargo build`

**Step 3: Full test suite**

Run: `cargo test -- --test-threads=8`

**Step 4: Commit**

```bash
git add -A
git commit -m "chore: pass fmt clippy build and tests for REST notes store"
```

---

## Server-side handoff notes (separate repo)

- Key by `(repo_url_normalized, commit_sha)`.
- Normalize repo URL on server before upsert/query.
- For upsert, use dialect-aware path (`on_conflict_do_update` where available; fallback approach for Oracle migration).
- Keep auth with existing bearer/api-key conventions.

---

## Definition of done

- `notes_store` fully available in runtime config + CLI config command + test patch.
- REST mode only changes remote sync channel; local note storage remains git-notes.
- Default mode remains current git behavior with no regression.
- All tests and quality gates pass.

---

Plan complete and saved to `docs/plans/2026-03-29-rest-notes-store-implementation-plan.md`. Two execution options:

**1. Subagent-Driven (this session)** - I dispatch fresh subagent per task, review between tasks, fast iteration

**2. Parallel Session (separate)** - Open new session with executing-plans, batch execution with checkpoints

Which approach?
