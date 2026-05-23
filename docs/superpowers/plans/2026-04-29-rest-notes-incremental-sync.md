# REST Git Notes Incremental Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement strong incremental REST synchronization for Git AI authorship notes using remote `change_seq` watermarks and stable SHA-256 content hashes.

**Architecture:** Extend the existing REST API types in `src/api/types.rs` with backward-compatible optional fields, then update `src/git/sync_authorship.rs` so REST fetch paginates by `since_change_seq`, verifies note `content_hash`, and only advances a local state file after every page and write succeeds. REST push remains a one-shot local-to-remote operation, but compares local SHA-256 note content against remote list summaries instead of relying on existence or Git blob OIDs.

**Tech Stack:** Rust 2024, serde, sha2, Git CLI-backed refs helpers in `src/git/refs.rs`, raw TCP mock API test pattern from `tests/daemon_mode.rs`, repository integration helpers from `tests/integration/repos/`.

---

## Source Specification

Implement the client-side design from `docs/2026-04-28-git-notes-rest-sync-design.md`.

Hard constraints for implementers:
- Do not change hook entry points: `fetch_authorship_notes()`, `push_authorship_notes()`, `fetch_and_merge_tracking_notes()`, clone/fetch/pull/push hooks, and `git ai fetch-notes` must continue calling the same top-level functions.
- Do not change Git notes storage ref; continue writing authorship notes to `refs/notes/ai`.
- Do not advance REST fetch state when any list page, batch fetch, hash validation, JSON parse, network call, or Git notes write fails.
- Do not use `note_blob_oid` for REST consistency decisions; keep it only for API compatibility and push payloads.
- Do not commit automatically while executing this plan unless the user explicitly asks for commits. The “commit” steps below are checkpoints for humans or sessions where commits were explicitly requested.

---

## File Structure

### Modify: `src/api/types.rs`

Responsibility: REST request/response schemas and serde backward compatibility tests.

Required changes:
- Add `AuthorshipNotesListItem`.
- Extend `AuthorshipNotesListRequest` with optional `since_change_seq` and `limit`.
- Extend `AuthorshipNotesListData` with optional `items`, `next_change_seq`, and `has_more`.
- Extend `AuthorshipNotesBatchItem` with optional `content_hash` and `change_seq`.
- Extend `AuthorshipNotesPushData` with optional `unchanged`.
- Add serde tests covering new and old response shapes.

### Modify: `src/git/sync_authorship.rs`

Responsibility: REST fetch/push orchestration, local state file management, SHA-256 content hash normalization, and unit tests for sync helpers.

Required changes:
- Add SHA-256 helper `sha256_note_content(content: &str) -> String`.
- Add hash parser `normalize_content_hash(hash: &str) -> Result<String, GitAiError>` accepting `sha256:<64-hex>` and `<64-hex>`.
- Add `RestNotesSyncState` and helpers for `.git/ai/rest_notes_sync_state/<repo-key>.json`.
- Add atomic state write using same-directory temp file + `sync_all()` + `rename()`.
- Rewrite `rest_fetch_authorship_notes()` to use incremental list items when present and fall back to the existing full-list path when `items` is absent.
- Rewrite `rest_push_notes()` to compute local note hashes and compare against remote list summaries.

### Modify: `src/api/authorship_notes.rs`

Responsibility: endpoint wrappers. Expected code changes are minimal because serde handles new fields, but keep this file in the verification set because request shapes change.

### Modify: `tests/daemon_mode.rs` or Create: `tests/rest_notes_sync.rs`

Responsibility: raw TCP mock REST API integration coverage.

Preferred approach: create `tests/rest_notes_sync.rs` with a focused mock server instead of broadening `tests/daemon_mode.rs`; copy the existing raw `TcpListener` style from `tests/daemon_mode.rs` so no new test dependency is needed.

### Existing helpers to reuse

- `src/git/refs.rs`: `notes_add_batch()`, `show_authorship_note()`, `note_blob_oids_for_commits()`, `list_all_notes()`.
- `src/repo_url.rs`: `normalize_repo_url()`.
- `tests/integration/repos/test_repo.rs`: `TestRepo`, `patch_git_ai_config()`, `read_authorship_note()`.
- `ConfigPatch.notes_store = Some("rest".to_string())` plus `GIT_AI_API_BASE_URL` for mock API tests.

---

## Task 1: Extend API Types for Incremental REST Notes

**Files:**
- Modify: `src/api/types.rs`
- Verify: `src/api/authorship_notes.rs`

- [ ] **Step 1: Write failing serde tests for new and old list response shapes**

Add these tests inside the existing `#[cfg(test)] mod tests` in `src/api/types.rs`:

```rust
#[test]
fn test_notes_list_request_serializes_incremental_fields() {
    let request = AuthorshipNotesListRequest {
        repo_url: "https://github.com/example/repo".to_string(),
        since_commit_time: None,
        since_change_seq: Some(42),
        limit: Some(1000),
    };

    let json = serde_json::to_value(&request).unwrap();
    assert_eq!(json["repo_url"], "https://github.com/example/repo");
    assert_eq!(json["since_change_seq"], 42);
    assert_eq!(json["limit"], 1000);
    assert!(json.get("since_commit_time").is_some());
}

#[test]
fn test_notes_list_response_deserializes_incremental_items() {
    let body = r#"
    {
        "ok": true,
        "data": {
            "commit_shas": ["abc123"],
            "items": [
                {
                    "commit_sha": "abc123",
                    "content_hash": "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                    "change_seq": 7,
                    "updated_at": 1775973635847
                }
            ],
            "next_change_seq": 7,
            "has_more": false
        }
    }
    "#;

    let parsed: AuthorshipNotesListResponse = serde_json::from_str(body).unwrap();
    let items = parsed.data.items.expect("items should deserialize");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].commit_sha, "abc123");
    assert_eq!(items[0].change_seq, 7);
    assert_eq!(parsed.data.next_change_seq, Some(7));
    assert_eq!(parsed.data.has_more, Some(false));
}

#[test]
fn test_notes_list_response_without_items_remains_compatible() {
    let body = r#"
    {
        "ok": true,
        "data": {
            "commit_shas": ["abc123"],
            "note_blob_oids": ["def456"]
        }
    }
    "#;

    let parsed: AuthorshipNotesListResponse = serde_json::from_str(body).unwrap();
    assert_eq!(parsed.data.commit_shas, vec!["abc123".to_string()]);
    assert_eq!(parsed.data.note_blob_oids, Some(vec!["def456".to_string()]));
    assert!(parsed.data.items.is_none());
    assert!(parsed.data.next_change_seq.is_none());
    assert!(parsed.data.has_more.is_none());
}
```

- [ ] **Step 2: Write failing serde tests for batch and push compatibility**

Add these tests in the same test module:

```rust
#[test]
fn test_notes_batch_item_deserializes_incremental_metadata() {
    let body = r#"
    {
        "ok": true,
        "data": {
            "notes": [
                {
                    "commit_sha": "abc123",
                    "content": "{\"version\":\"authorship/3.0.0\"}",
                    "note_blob_oid": "blob123",
                    "content_hash": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                    "change_seq": 9
                }
            ],
            "missing": []
        }
    }
    "#;

    let parsed: AuthorshipBatchResponse = serde_json::from_str(body).unwrap();
    let note = &parsed.data.notes[0];
    assert_eq!(note.commit_sha, "abc123");
    assert_eq!(note.content_hash.as_deref(), Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"));
    assert_eq!(note.change_seq, Some(9));
}

#[test]
fn test_notes_batch_item_without_incremental_metadata_remains_compatible() {
    let body = r#"
    {
        "ok": true,
        "data": {
            "notes": [
                {
                    "commit_sha": "abc123",
                    "content": "{}",
                    "note_blob_oid": null
                }
            ],
            "missing": []
        }
    }
    "#;

    let parsed: AuthorshipBatchResponse = serde_json::from_str(body).unwrap();
    let note = &parsed.data.notes[0];
    assert!(note.content_hash.is_none());
    assert!(note.change_seq.is_none());
}

#[test]
fn test_notes_push_response_deserializes_optional_unchanged() {
    let body = r#"
    {
        "ok": true,
        "data": {
            "created": 1,
            "updated": 2,
            "unchanged": 3
        }
    }
    "#;

    let parsed: AuthorshipNotesPushResponse = serde_json::from_str(body).unwrap();
    assert_eq!(parsed.data.created, 1);
    assert_eq!(parsed.data.updated, 2);
    assert_eq!(parsed.data.unchanged, Some(3));
}

#[test]
fn test_notes_push_response_without_unchanged_remains_compatible() {
    let body = r#"
    {
        "ok": true,
        "data": {
            "created": 1,
            "updated": 2
        }
    }
    "#;

    let parsed: AuthorshipNotesPushResponse = serde_json::from_str(body).unwrap();
    assert_eq!(parsed.data.created, 1);
    assert_eq!(parsed.data.updated, 2);
    assert!(parsed.data.unchanged.is_none());
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run:

```bash
task test TEST_FILTER=api::types::tests
```

Expected: FAIL with compiler errors mentioning missing fields or type `AuthorshipNotesListItem`.

- [ ] **Step 4: Implement the type extensions**

Update the REST notes section in `src/api/types.rs` to this shape, preserving existing derives and public visibility:

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuthorshipNotesListRequest {
    pub repo_url: String,
    pub since_commit_time: Option<i64>,
    #[serde(default)]
    pub since_change_seq: Option<i64>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct AuthorshipNotesListItem {
    pub commit_sha: String,
    pub content_hash: String,
    pub change_seq: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuthorshipNotesListData {
    pub commit_shas: Vec<String>,
    pub note_blob_oids: Option<Vec<String>>,
    #[serde(default)]
    pub items: Option<Vec<AuthorshipNotesListItem>>,
    #[serde(default)]
    pub next_change_seq: Option<i64>,
    #[serde(default)]
    pub has_more: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuthorshipNotesBatchItem {
    pub commit_sha: String,
    pub content: String,
    pub note_blob_oid: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
    #[serde(default)]
    pub change_seq: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AuthorshipNotesPushData {
    pub created: usize,
    pub updated: usize,
    #[serde(default)]
    pub unchanged: Option<usize>,
}
```

If the file already has these structs, edit only the missing fields and derives.

- [ ] **Step 5: Update all `AuthorshipNotesListRequest` construction sites**

Update every construction in `src/git/sync_authorship.rs` and tests to include:

```rust
since_change_seq: None,
limit: None,
```

For incremental fetch and push tasks later, these values will become `Some(...)`.

- [ ] **Step 6: Run type tests to verify pass**

Run:

```bash
task test TEST_FILTER=api::types::tests
```

Expected: PASS.

- [ ] **Step 7: Optional commit checkpoint**

Only if the user explicitly requested commits:

```bash
git add src/api/types.rs src/git/sync_authorship.rs
git commit -m "feat: extend authorship notes REST types"
```

---

## Task 2: Add Hash and REST Sync State Helpers

**Files:**
- Modify: `src/git/sync_authorship.rs`

- [ ] **Step 1: Write failing tests for note content hashing**

Add tests inside `#[cfg(test)] mod tests` in `src/git/sync_authorship.rs`:

```rust
#[test]
fn test_sha256_note_content_hashes_raw_text() {
    assert_eq!(
        sha256_note_content("hello\n"),
        "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"
    );
}

#[test]
fn test_normalize_content_hash_accepts_prefixed_and_plain_hex() {
    let hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    assert_eq!(normalize_content_hash(hash).unwrap(), hash);
    assert_eq!(
        normalize_content_hash("sha256:0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF").unwrap(),
        hash
    );
}

#[test]
fn test_normalize_content_hash_rejects_invalid_values() {
    assert!(normalize_content_hash("sha1:0123").is_err());
    assert!(normalize_content_hash("sha256:not-hex").is_err());
    assert!(normalize_content_hash("0123").is_err());
}
```

- [ ] **Step 2: Write failing tests for state path and stale repo mismatch**

Add tests using `tempfile::TempDir` or existing test temp patterns:

```rust
#[test]
fn test_rest_notes_sync_state_path_uses_hash_key() {
    let tmp = tempfile::tempdir().unwrap();
    let git_dir = tmp.path().join(".git");
    std::fs::create_dir_all(&git_dir).unwrap();

    let path = rest_notes_sync_state_path(&git_dir, "https://github.com/example/repo");
    assert!(path.starts_with(git_dir.join("ai").join("rest_notes_sync_state")));
    let filename = path.file_name().unwrap().to_string_lossy();
    assert!(filename.ends_with(".json"));
    assert!(!filename.contains('/'));
    assert!(!filename.contains(':'));
}

#[test]
fn test_read_rest_notes_sync_state_ignores_repo_url_mismatch() {
    let tmp = tempfile::tempdir().unwrap();
    let git_dir = tmp.path().join(".git");
    std::fs::create_dir_all(git_dir.join("ai").join("rest_notes_sync_state")).unwrap();

    let path = rest_notes_sync_state_path(&git_dir, "https://github.com/example/repo");
    std::fs::write(
        &path,
        r#"{"schema_version":1,"repo_url":"https://github.com/other/repo","last_change_seq":99,"updated_at":1}"#,
    )
    .unwrap();

    let state = read_rest_notes_sync_state(&git_dir, "https://github.com/example/repo").unwrap();
    assert_eq!(state.last_change_seq, 0);
    assert_eq!(state.repo_url, "https://github.com/example/repo");
}
```

- [ ] **Step 3: Write failing test for monotonic atomic state update**

Add:

```rust
#[test]
fn test_write_rest_notes_sync_state_never_moves_watermark_backwards() {
    let tmp = tempfile::tempdir().unwrap();
    let git_dir = tmp.path().join(".git");
    std::fs::create_dir_all(&git_dir).unwrap();
    let repo_url = "https://github.com/example/repo";

    write_rest_notes_sync_state(&git_dir, repo_url, 10).unwrap();
    write_rest_notes_sync_state(&git_dir, repo_url, 7).unwrap();

    let state = read_rest_notes_sync_state(&git_dir, repo_url).unwrap();
    assert_eq!(state.last_change_seq, 10);
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run:

```bash
task test TEST_FILTER=sync_authorship::tests
```

Expected: FAIL with missing helper functions and imports.

- [ ] **Step 5: Implement hash helpers and state struct**

Add imports near the top of `src/git/sync_authorship.rs`:

```rust
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
```

If any imports already exist, merge them rather than duplicating.

Add helper code near the REST sync functions:

```rust
const REST_NOTES_SYNC_STATE_SCHEMA_VERSION: u32 = 1;
const REST_NOTES_SYNC_LIST_LIMIT: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct RestNotesSyncState {
    schema_version: u32,
    repo_url: String,
    last_change_seq: i64,
    updated_at: i64,
}

fn sha256_note_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn normalize_content_hash(hash: &str) -> Result<String, GitAiError> {
    let raw = hash.strip_prefix("sha256:").unwrap_or(hash).to_ascii_lowercase();
    if raw.len() == 64 && raw.chars().all(|ch| ch.is_ascii_hexdigit()) {
        Ok(raw)
    } else {
        Err(GitAiError::Generic(format!("invalid authorship note content hash: {hash}")))
    }
}

fn rest_notes_repo_key(repo_url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repo_url.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn rest_notes_sync_state_path(git_dir: &Path, repo_url: &str) -> PathBuf {
    git_dir
        .join("ai")
        .join("rest_notes_sync_state")
        .join(format!("{}.json", rest_notes_repo_key(repo_url)))
}

fn current_time_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn default_rest_notes_sync_state(repo_url: &str) -> RestNotesSyncState {
    RestNotesSyncState {
        schema_version: REST_NOTES_SYNC_STATE_SCHEMA_VERSION,
        repo_url: repo_url.to_string(),
        last_change_seq: 0,
        updated_at: 0,
    }
}

fn read_rest_notes_sync_state(git_dir: &Path, repo_url: &str) -> Result<RestNotesSyncState, GitAiError> {
    let path = rest_notes_sync_state_path(git_dir, repo_url);
    if !path.exists() {
        return Ok(default_rest_notes_sync_state(repo_url));
    }

    let raw = fs::read_to_string(&path)?;
    let state: RestNotesSyncState = serde_json::from_str(&raw)?;
    if state.schema_version != REST_NOTES_SYNC_STATE_SCHEMA_VERSION || state.repo_url != repo_url {
        return Ok(default_rest_notes_sync_state(repo_url));
    }
    Ok(state)
}

fn write_rest_notes_sync_state(git_dir: &Path, repo_url: &str, last_change_seq: i64) -> Result<(), GitAiError> {
    let path = rest_notes_sync_state_path(git_dir, repo_url);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let existing = read_rest_notes_sync_state(git_dir, repo_url)?;
    let next_last_change_seq = existing.last_change_seq.max(last_change_seq);
    let state = RestNotesSyncState {
        schema_version: REST_NOTES_SYNC_STATE_SCHEMA_VERSION,
        repo_url: repo_url.to_string(),
        last_change_seq: next_last_change_seq,
        updated_at: current_time_millis(),
    };

    let tmp_path = path.with_extension(format!("json.tmp.{}", std::process::id()));
    let mut file = fs::File::create(&tmp_path)?;
    file.write_all(serde_json::to_vec_pretty(&state)?.as_slice())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(tmp_path, path)?;
    Ok(())
}
```

- [ ] **Step 6: Run helper tests to verify pass**

Run:

```bash
task test TEST_FILTER=sync_authorship::tests
```

Expected: PASS.

- [ ] **Step 7: Optional commit checkpoint**

Only if the user explicitly requested commits:

```bash
git add src/git/sync_authorship.rs
git commit -m "feat: add REST notes sync state helpers"
```

---

## Task 3: Implement Incremental REST Fetch with Full-List Fallback

**Files:**
- Modify: `src/git/sync_authorship.rs`
- Test: `src/git/sync_authorship.rs` unit tests
- Test: `tests/rest_notes_sync.rs`

- [ ] **Step 1: Extract old fetch path into a fallback helper**

Before changing behavior, move the body of the existing `rest_fetch_authorship_notes()` full-list implementation into a helper named:

```rust
fn rest_fetch_authorship_notes_legacy_full_list(
    repo: &Repository,
    client: &ApiClient,
    normalized_repo_url: &str,
    response: AuthorshipNotesListResponse,
) -> Result<AuthorshipSyncResult, GitAiError>
```

The helper should use `response.data.commit_shas`, `note_blob_oids_for_commits()`, `authorship_notes_batch_get()`, and `notes_add_batch()` exactly like the current code.

- [ ] **Step 2: Run existing sync tests after extraction**

Run:

```bash
task test TEST_FILTER=sync_authorship::tests
```

Expected: PASS. This confirms the extraction did not alter behavior.

- [ ] **Step 3: Write failing unit test for page validation**

Add a pure helper and tests before full HTTP integration. Desired helper signature:

```rust
fn validate_next_change_seq(
    items: &[AuthorshipNotesListItem],
    next_change_seq: Option<i64>,
) -> Result<i64, GitAiError>
```

Add tests:

```rust
#[test]
fn test_validate_next_change_seq_uses_page_maximum() {
    let items = vec![AuthorshipNotesListItem {
        commit_sha: "abc123".to_string(),
        content_hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        change_seq: 5,
        updated_at: 1,
    }];

    assert_eq!(validate_next_change_seq(&items, Some(5)).unwrap(), 5);
    assert_eq!(validate_next_change_seq(&items, Some(8)).unwrap(), 8);
}

#[test]
fn test_validate_next_change_seq_rejects_backwards_value() {
    let items = vec![AuthorshipNotesListItem {
        commit_sha: "abc123".to_string(),
        content_hash: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        change_seq: 5,
        updated_at: 1,
    }];

    assert!(validate_next_change_seq(&items, Some(4)).is_err());
}
```

- [ ] **Step 4: Implement `validate_next_change_seq()`**

Add:

```rust
fn validate_next_change_seq(
    items: &[AuthorshipNotesListItem],
    next_change_seq: Option<i64>,
) -> Result<i64, GitAiError> {
    let max_item_change_seq = items.iter().map(|item| item.change_seq).max().unwrap_or(0);
    let next = next_change_seq.unwrap_or(max_item_change_seq);
    if next < max_item_change_seq {
        return Err(GitAiError::Generic(format!(
            "authorship notes list next_change_seq {next} is behind page maximum {max_item_change_seq}"
        )));
    }
    Ok(next)
}
```

- [ ] **Step 5: Create focused REST notes mock integration test file**

Create `tests/rest_notes_sync.rs` with a raw TCP mock. Use this skeleton and keep it focused on authorship notes:

```rust
mod integration;

use integration::repos::{TestRepo, lines};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

struct ScopedEnvVar {
    key: &'static str,
    previous: Option<String>,
}

impl ScopedEnvVar {
    fn set(key: &'static str, value: String) -> Self {
        let previous = std::env::var(key).ok();
        unsafe { std::env::set_var(key, value); }
        Self { key, previous }
    }
}

impl Drop for ScopedEnvVar {
    fn drop(&mut self) {
        unsafe {
            if let Some(previous) = &self.previous {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
}

#[derive(Default)]
struct RestNotesMockState {
    responses: VecDeque<Value>,
    requests: Vec<Value>,
    paths: Vec<String>,
}

struct RestNotesMockServer {
    base_url: String,
    state: Arc<Mutex<RestNotesMockState>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl RestNotesMockServer {
    fn start(responses: Vec<Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let state = Arc::new(Mutex::new(RestNotesMockState {
            responses: VecDeque::from(responses),
            requests: Vec::new(),
            paths: Vec::new(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_state = Arc::clone(&state);
        let thread_stop = Arc::clone(&stop);

        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => handle_rest_notes_connection(stream, &thread_state),
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            base_url: format!("http://{addr}"),
            state,
            stop,
            thread: Some(thread),
        }
    }

    fn base_url(&self) -> String {
        self.base_url.clone()
    }

    fn paths(&self) -> Vec<String> {
        self.state.lock().unwrap().paths.clone()
    }

    fn requests(&self) -> Vec<Value> {
        self.state.lock().unwrap().requests.clone()
    }
}

impl Drop for RestNotesMockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(self.base_url.trim_start_matches("http://"));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle_rest_notes_connection(mut stream: TcpStream, state: &Arc<Mutex<RestNotesMockState>>) {
    let mut buffer = Vec::new();
    let mut temp = [0_u8; 4096];
    loop {
        match stream.read(&mut temp) {
            Ok(0) => break,
            Ok(n) => {
                buffer.extend_from_slice(&temp[..n]);
                if let Some(header_end) = find_header_end(&buffer) {
                    let headers = String::from_utf8_lossy(&buffer[..header_end]);
                    let content_length = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("Content-Length:"))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    let body_start = header_end + 4;
                    if buffer.len() >= body_start + content_length {
                        break;
                    }
                }
            }
            Err(_) => return,
        }
    }

    let request_text = String::from_utf8_lossy(&buffer);
    let path = request_text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/")
        .to_string();
    let body = request_text
        .split("\r\n\r\n")
        .nth(1)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or(Value::Null);

    let response = {
        let mut locked = state.lock().unwrap();
        locked.paths.push(path);
        locked.requests.push(body);
        locked.responses.pop_front().unwrap_or_else(|| json!({"ok": false, "error": "unexpected request"}))
    };

    let response_body = response.to_string();
    let status = if response.get("ok").and_then(Value::as_bool) == Some(false) { 500 } else { 200 };
    let raw = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    let _ = stream.write_all(raw.as_bytes());
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn sha256_text(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}
```

- [ ] **Step 6: Write failing integration test for missing local note fetch**

Append to `tests/rest_notes_sync.rs`:

```rust
#[test]
fn rest_fetch_downloads_missing_note_and_advances_watermark() {
    let repo = TestRepo::new();
    let mut file = repo.filename("example.txt");
    file.set_contents(lines!["AI line".ai()]);
    repo.stage_all_and_commit("initial").unwrap();
    let commit_sha = repo.head_sha();
    let note_content = repo.read_authorship_note(&commit_sha).expect("note should exist before deletion");
    repo.git_og(&["notes", "--ref=ai", "remove", &commit_sha]).unwrap();
    assert!(repo.read_authorship_note(&commit_sha).is_none());

    let hash = sha256_text(&note_content);
    let server = RestNotesMockServer::start(vec![
        json!({
            "ok": true,
            "data": {
                "commit_shas": [commit_sha],
                "items": [{
                    "commit_sha": commit_sha,
                    "content_hash": format!("sha256:{hash}"),
                    "change_seq": 1,
                    "updated_at": 1775973635847_i64
                }],
                "next_change_seq": 1,
                "has_more": false
            }
        }),
        json!({
            "ok": true,
            "data": {
                "notes": [{
                    "commit_sha": commit_sha,
                    "content": note_content,
                    "note_blob_oid": null,
                    "content_hash": hash,
                    "change_seq": 1
                }],
                "missing": []
            }
        }),
    ]);
    let _api_url = ScopedEnvVar::set("GIT_AI_API_BASE_URL", server.base_url());
    repo.patch_git_ai_config(|patch| {
        patch.notes_store = Some("rest".to_string());
    });

    repo.git_ai(&["fetch-notes", "--remote", "origin"]).unwrap();

    let restored = repo.read_authorship_note(&commit_sha).expect("REST fetch should restore note");
    assert_eq!(restored, note_content);
    assert_eq!(server.paths(), vec!["/worker/authorship_notes/list", "/worker/authorship_notes/batch"]);
    assert_eq!(server.requests()[0]["since_change_seq"], 0);
    assert_eq!(server.requests()[0]["limit"], 1000);
}
```

If `TestRepo` has no `head_sha()` helper, use the existing helper or replace with:

```rust
let commit_sha = repo.git_og(&["rev-parse", "HEAD"]).unwrap().trim().to_string();
```

- [ ] **Step 7: Write failing integration test for same hash skipping batch**

Add:

```rust
#[test]
fn rest_fetch_skips_batch_when_local_hash_matches() {
    let repo = TestRepo::new();
    let mut file = repo.filename("example.txt");
    file.set_contents(lines!["AI line".ai()]);
    repo.stage_all_and_commit("initial").unwrap();
    let commit_sha = repo.git_og(&["rev-parse", "HEAD"]).unwrap().trim().to_string();
    let note_content = repo.read_authorship_note(&commit_sha).unwrap();
    let hash = sha256_text(&note_content);

    let server = RestNotesMockServer::start(vec![json!({
        "ok": true,
        "data": {
            "commit_shas": [commit_sha],
            "items": [{
                "commit_sha": commit_sha,
                "content_hash": hash,
                "change_seq": 2,
                "updated_at": 1775973635847_i64
            }],
            "next_change_seq": 2,
            "has_more": false
        }
    })]);
    let _api_url = ScopedEnvVar::set("GIT_AI_API_BASE_URL", server.base_url());
    repo.patch_git_ai_config(|patch| {
        patch.notes_store = Some("rest".to_string());
    });

    repo.git_ai(&["fetch-notes", "--remote", "origin"]).unwrap();

    assert_eq!(server.paths(), vec!["/worker/authorship_notes/list"]);
}
```

- [ ] **Step 8: Write failing integration test for hash mismatch failure preserving watermark**

Add:

```rust
#[test]
fn rest_fetch_rejects_batch_hash_mismatch_and_does_not_advance_watermark() {
    let repo = TestRepo::new();
    let mut file = repo.filename("example.txt");
    file.set_contents(lines!["AI line".ai()]);
    repo.stage_all_and_commit("initial").unwrap();
    let commit_sha = repo.git_og(&["rev-parse", "HEAD"]).unwrap().trim().to_string();
    let note_content = repo.read_authorship_note(&commit_sha).unwrap();
    repo.git_og(&["notes", "--ref=ai", "remove", &commit_sha]).unwrap();

    let wrong_hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let server = RestNotesMockServer::start(vec![
        json!({
            "ok": true,
            "data": {
                "commit_shas": [commit_sha],
                "items": [{
                    "commit_sha": commit_sha,
                    "content_hash": wrong_hash,
                    "change_seq": 3,
                    "updated_at": 1775973635847_i64
                }],
                "next_change_seq": 3,
                "has_more": false
            }
        }),
        json!({
            "ok": true,
            "data": {
                "notes": [{
                    "commit_sha": commit_sha,
                    "content": note_content,
                    "note_blob_oid": null,
                    "content_hash": wrong_hash,
                    "change_seq": 3
                }],
                "missing": []
            }
        }),
    ]);
    let _api_url = ScopedEnvVar::set("GIT_AI_API_BASE_URL", server.base_url());
    repo.patch_git_ai_config(|patch| {
        patch.notes_store = Some("rest".to_string());
    });

    let result = repo.git_ai(&["fetch-notes", "--remote", "origin"]);
    assert!(result.is_err());
    assert!(repo.read_authorship_note(&commit_sha).is_none());
}
```

- [ ] **Step 9: Run integration tests to verify they fail**

Run:

```bash
task test TEST_FILTER=rest_notes_sync
```

Expected: FAIL because incremental fetch is not implemented and request fields/state behavior are missing.

- [ ] **Step 10: Implement incremental fetch loop**

Rewrite `rest_fetch_authorship_notes()` to follow this structure:

```rust
fn rest_fetch_authorship_notes(repo: &Repository, remote: &str) -> Result<AuthorshipSyncResult, GitAiError> {
    let remote_url = fetch_remote_from_args(repo, remote)?;
    let normalized_repo_url = normalize_repo_url(&remote_url)?;
    let client = build_api_client()?;
    let git_dir = repo.path();
    let state = read_rest_notes_sync_state(git_dir, &normalized_repo_url)?;
    let mut since_change_seq = state.last_change_seq;
    let mut final_change_seq = state.last_change_seq;
    let mut total_written = 0_usize;

    loop {
        let response = client.authorship_notes_list(&AuthorshipNotesListRequest {
            repo_url: normalized_repo_url.clone(),
            since_commit_time: None,
            since_change_seq: Some(since_change_seq),
            limit: Some(REST_NOTES_SYNC_LIST_LIMIT),
        })?;

        let Some(items) = response.data.items else {
            return rest_fetch_authorship_notes_legacy_full_list(repo, &client, &normalized_repo_url, response);
        };

        if items.is_empty() {
            break;
        }

        let page_next_change_seq = validate_next_change_seq(&items, response.data.next_change_seq)?;
        let mut expected_hash_by_commit = std::collections::HashMap::new();
        let mut to_fetch = Vec::new();

        for item in &items {
            let expected_hash = normalize_content_hash(&item.content_hash)?;
            expected_hash_by_commit.insert(item.commit_sha.clone(), expected_hash.clone());
            match show_authorship_note(repo, &item.commit_sha)? {
                Some(local_content) if sha256_note_content(&local_content) == expected_hash => {}
                Some(_) | None => to_fetch.push(item.commit_sha.clone()),
            }
        }

        if !to_fetch.is_empty() {
            let batch = client.authorship_notes_batch_get(&AuthorshipNotesBatchRequest {
                repo_url: normalized_repo_url.clone(),
                commit_shas: to_fetch.clone(),
            })?;

            let missing_from_page: Vec<String> = batch
                .data
                .missing
                .iter()
                .filter(|commit_sha| expected_hash_by_commit.contains_key(*commit_sha))
                .cloned()
                .collect();
            if !missing_from_page.is_empty() {
                return Err(GitAiError::Generic(format!(
                    "authorship notes batch missing commits from list page: {}",
                    missing_from_page.join(", ")
                )));
            }

            let mut notes_to_write = Vec::new();
            for note in batch.data.notes {
                let Some(expected_hash) = expected_hash_by_commit.get(&note.commit_sha) else {
                    continue;
                };
                let actual_hash = sha256_note_content(&note.content);
                if &actual_hash != expected_hash {
                    return Err(GitAiError::Generic(format!(
                        "authorship note content hash mismatch for {}: expected {}, got {}",
                        note.commit_sha, expected_hash, actual_hash
                    )));
                }
                notes_to_write.push((note.commit_sha, note.content));
            }

            if !notes_to_write.is_empty() {
                notes_add_batch(repo, &notes_to_write)?;
                total_written += notes_to_write.len();
            }
        }

        final_change_seq = page_next_change_seq;
        since_change_seq = page_next_change_seq;
        if response.data.has_more != Some(true) {
            break;
        }
    }

    if final_change_seq > state.last_change_seq {
        write_rest_notes_sync_state(git_dir, &normalized_repo_url, final_change_seq)?;
    }

    if total_written > 0 {
        Ok(AuthorshipSyncResult::Found)
    } else {
        Ok(AuthorshipSyncResult::NotFound)
    }
}
```

Adjust `build_api_client()` or existing client construction to match the current code; do not invent a second API client setup path if one already exists in the file.

- [ ] **Step 11: Run fetch tests**

Run:

```bash
task test TEST_FILTER=sync_authorship::tests
task test TEST_FILTER=rest_notes_sync
task test TEST_FILTER=fetch_notes
```

Expected: PASS.

- [ ] **Step 12: Optional commit checkpoint**

Only if the user explicitly requested commits:

```bash
git add src/git/sync_authorship.rs tests/rest_notes_sync.rs
git commit -m "feat: fetch REST authorship notes incrementally"
```

---

## Task 4: Implement Hash-Based REST Push

**Files:**
- Modify: `src/git/sync_authorship.rs`
- Test: `tests/rest_notes_sync.rs`

- [ ] **Step 1: Write failing integration test for same remote hash skipping push**

Append to `tests/rest_notes_sync.rs`:

```rust
#[test]
fn rest_push_skips_note_when_remote_hash_matches() {
    let repo = TestRepo::new();
    let mut file = repo.filename("example.txt");
    file.set_contents(lines!["AI line".ai()]);
    repo.stage_all_and_commit("initial").unwrap();
    let commit_sha = repo.git_og(&["rev-parse", "HEAD"]).unwrap().trim().to_string();
    let note_content = repo.read_authorship_note(&commit_sha).unwrap();
    let hash = sha256_text(&note_content);

    let server = RestNotesMockServer::start(vec![json!({
        "ok": true,
        "data": {
            "commit_shas": [commit_sha],
            "items": [{
                "commit_sha": commit_sha,
                "content_hash": hash,
                "change_seq": 11,
                "updated_at": 1775973635847_i64
            }],
            "next_change_seq": 11,
            "has_more": false
        }
    })]);
    let _api_url = ScopedEnvVar::set("GIT_AI_API_BASE_URL", server.base_url());
    repo.patch_git_ai_config(|patch| {
        patch.notes_store = Some("rest".to_string());
    });

    repo.git_ai(&["internal", "push-authorship-notes", "origin"]).unwrap();

    assert_eq!(server.paths(), vec!["/worker/authorship_notes/list"]);
}
```

If the internal command name differs, inspect `tests/integration/internal_machine_commands.rs` and use the existing command shape that calls `push_authorship_notes()`.

- [ ] **Step 2: Write failing integration test for different remote hash pushing update**

Add:

```rust
#[test]
fn rest_push_sends_note_when_remote_hash_differs() {
    let repo = TestRepo::new();
    let mut file = repo.filename("example.txt");
    file.set_contents(lines!["AI line".ai()]);
    repo.stage_all_and_commit("initial").unwrap();
    let commit_sha = repo.git_og(&["rev-parse", "HEAD"]).unwrap().trim().to_string();

    let old_hash = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let server = RestNotesMockServer::start(vec![
        json!({
            "ok": true,
            "data": {
                "commit_shas": [commit_sha],
                "items": [{
                    "commit_sha": commit_sha,
                    "content_hash": old_hash,
                    "change_seq": 12,
                    "updated_at": 1775973635847_i64
                }],
                "next_change_seq": 12,
                "has_more": false
            }
        }),
        json!({
            "ok": true,
            "data": {
                "created": 0,
                "updated": 1,
                "unchanged": 0
            }
        }),
    ]);
    let _api_url = ScopedEnvVar::set("GIT_AI_API_BASE_URL", server.base_url());
    repo.patch_git_ai_config(|patch| {
        patch.notes_store = Some("rest".to_string());
    });

    repo.git_ai(&["internal", "push-authorship-notes", "origin"]).unwrap();

    assert_eq!(server.paths(), vec!["/worker/authorship_notes/list", "/worker/authorship_notes/push"]);
    let push_body = &server.requests()[1];
    assert_eq!(push_body["notes"].as_array().unwrap().len(), 1);
    assert_eq!(push_body["notes"][0]["commit_sha"], commit_sha);
}
```

- [ ] **Step 3: Write failing integration test for missing remote commit pushing create**

Add:

```rust
#[test]
fn rest_push_sends_note_when_remote_commit_missing() {
    let repo = TestRepo::new();
    let mut file = repo.filename("example.txt");
    file.set_contents(lines!["AI line".ai()]);
    repo.stage_all_and_commit("initial").unwrap();

    let server = RestNotesMockServer::start(vec![
        json!({
            "ok": true,
            "data": {
                "commit_shas": [],
                "items": [],
                "next_change_seq": 0,
                "has_more": false
            }
        }),
        json!({
            "ok": true,
            "data": {
                "created": 1,
                "updated": 0
            }
        }),
    ]);
    let _api_url = ScopedEnvVar::set("GIT_AI_API_BASE_URL", server.base_url());
    repo.patch_git_ai_config(|patch| {
        patch.notes_store = Some("rest".to_string());
    });

    repo.git_ai(&["internal", "push-authorship-notes", "origin"]).unwrap();

    assert_eq!(server.paths(), vec!["/worker/authorship_notes/list", "/worker/authorship_notes/push"]);
    assert_eq!(server.requests()[1]["notes"].as_array().unwrap().len(), 1);
}
```

- [ ] **Step 4: Run push tests to verify they fail**

Run:

```bash
task test TEST_FILTER=rest_notes_sync
```

Expected: FAIL because current push only skips based on `commit_shas` existence and does not use content hashes.

- [ ] **Step 5: Implement remote summary collection helper**

Add helper in `src/git/sync_authorship.rs`:

```rust
fn fetch_remote_note_hashes(
    client: &ApiClient,
    normalized_repo_url: &str,
) -> Result<std::collections::HashMap<String, String>, GitAiError> {
    let mut since_change_seq = 0_i64;
    let mut remote_hash_by_commit = std::collections::HashMap::new();

    loop {
        let response = client.authorship_notes_list(&AuthorshipNotesListRequest {
            repo_url: normalized_repo_url.to_string(),
            since_commit_time: None,
            since_change_seq: Some(since_change_seq),
            limit: Some(REST_NOTES_SYNC_LIST_LIMIT),
        })?;

        let Some(items) = response.data.items else {
            for commit_sha in response.data.commit_shas {
                remote_hash_by_commit.insert(commit_sha, String::new());
            }
            return Ok(remote_hash_by_commit);
        };

        let page_next_change_seq = validate_next_change_seq(&items, response.data.next_change_seq)?;
        for item in items {
            remote_hash_by_commit.insert(item.commit_sha, normalize_content_hash(&item.content_hash)?);
        }

        since_change_seq = page_next_change_seq;
        if response.data.has_more != Some(true) {
            break;
        }
    }

    Ok(remote_hash_by_commit)
}
```

The empty string in the old-server fallback means “remote has this commit but hash is unknown”; preserve old behavior by skipping those commits unless they are absent.

- [ ] **Step 6: Rewrite push diff selection**

Inside `rest_push_notes()`:

1. Keep local note enumeration via existing `list_local_authorship_notes_with_blob_oid()` or equivalent.
2. Call `fetch_remote_note_hashes()`.
3. For each local `(commit_sha, note_blob_oid)`, read local content with `show_authorship_note()`.
4. Compute `local_hash = sha256_note_content(&content)`.
5. Push if:
   - remote map has no entry for commit; or
   - remote map entry is non-empty and differs from local hash.
6. Skip if:
   - remote map entry equals local hash; or
   - remote map entry is empty old-server fallback marker.
7. Keep existing `AuthorshipNotesPushItem` fields unchanged.
8. When printing/debugging response stats, include `unchanged.unwrap_or(0)` only if the code already logs created/updated.

- [ ] **Step 7: Run push tests**

Run:

```bash
task test TEST_FILTER=rest_notes_sync
task test TEST_FILTER=internal_machine_commands
task test TEST_FILTER=push_upstream
```

Expected: PASS.

- [ ] **Step 8: Optional commit checkpoint**

Only if the user explicitly requested commits:

```bash
git add src/git/sync_authorship.rs tests/rest_notes_sync.rs
git commit -m "feat: push REST authorship notes by content hash"
```

---

## Task 5: Cover Pagination, Failure Semantics, and Legacy Fallback

**Files:**
- Modify: `tests/rest_notes_sync.rs`
- Modify: `src/git/sync_authorship.rs` only if tests expose gaps

- [ ] **Step 1: Add test for multi-page fetch advancing to final watermark**

Add a test that returns two `/list` pages and one `/batch` per missing note. Use `has_more: true` on page 1, `has_more: false` on page 2, and assert:

```rust
assert_eq!(server.requests()[0]["since_change_seq"], 0);
assert_eq!(server.requests()[2]["since_change_seq"], 1);
```

Then run the same fetch again with an empty list response and assert the first request of the second run uses:

```rust
assert_eq!(server.requests()[4]["since_change_seq"], 2);
```

Use two distinct commits created in one `TestRepo`; read/delete both notes before fetch.

- [ ] **Step 2: Add test for second page failure preserving old watermark**

Add a test sequence:

1. First list page returns one item with `has_more: true`, batch succeeds.
2. Second list response returns HTTP 500 via mock response `{"ok": false, "error": "boom"}`.
3. Fetch command returns error.
4. Re-run fetch with a new mock response and assert the first request still uses `since_change_seq: 0`.

This proves state is only written after all pages succeed.

- [ ] **Step 3: Add test for legacy list fallback not writing watermark**

Add a test where `/list` response has `commit_shas` but no `items`, then `/batch` succeeds. Assert note is restored. Re-run fetch with incremental response and assert first incremental request still uses `since_change_seq: 0`.

- [ ] **Step 4: Add test for local hash mismatch overwriting existing local note**

Create an existing local note, then change it manually with:

```bash
git notes --ref=ai add -f -m '{"stale":true}' <commit_sha>
```

Run REST fetch with list item hash for the original remote note and batch returning the original remote note. Assert `read_authorship_note()` equals the remote note content after fetch.

- [ ] **Step 5: Run the expanded integration suite**

Run:

```bash
task test TEST_FILTER=rest_notes_sync
```

Expected: PASS.

- [ ] **Step 6: Fix any root-cause gaps**

If failures occur, fix the implementation rather than weakening assertions. Common expected fixes:
- Do not write state inside the pagination loop.
- Treat missing commits from the current list page as a hard error.
- Normalize both prefixed and plain hashes before comparing.
- Ensure old-server fallback does not call `write_rest_notes_sync_state()`.

- [ ] **Step 7: Optional commit checkpoint**

Only if the user explicitly requested commits:

```bash
git add src/git/sync_authorship.rs tests/rest_notes_sync.rs
git commit -m "test: cover REST notes sync pagination failures"
```

---

## Task 6: Final Verification and Regression Sweep

**Files:**
- Verify all modified files

- [ ] **Step 1: Run Rust diagnostics on modified files**

Use LSP diagnostics for:

```text
src/api/types.rs
src/api/authorship_notes.rs
src/git/sync_authorship.rs
tests/rest_notes_sync.rs
```

Expected: zero errors.

- [ ] **Step 2: Run formatting**

Run:

```bash
task fmt
```

Expected: exit code 0. If it modifies files, inspect the diff before proceeding.

- [ ] **Step 3: Run lint**

Run:

```bash
task lint
```

Expected: exit code 0.

- [ ] **Step 4: Run targeted tests**

Run:

```bash
task test TEST_FILTER=api::types::tests
task test TEST_FILTER=sync_authorship::tests
task test TEST_FILTER=rest_notes_sync
task test TEST_FILTER=fetch_notes
task test TEST_FILTER=internal_machine_commands
task test TEST_FILTER=push_upstream
```

Expected: all PASS.

- [ ] **Step 5: Run build**

Run:

```bash
task build
```

Expected: exit code 0.

- [ ] **Step 6: Run full test suite if targeted tests and build pass**

Run:

```bash
task test
```

Expected: all PASS. If full test runtime is too high for the current session, record that targeted tests, lint, fmt, and build passed, and identify full suite as the remaining verification.

- [ ] **Step 7: Review git diff for accidental scope creep**

Run:

```bash
git diff -- src/api/types.rs src/api/authorship_notes.rs src/git/sync_authorship.rs tests/rest_notes_sync.rs
```

Expected: only REST authorship notes incremental sync changes and tests.

- [ ] **Step 8: Optional final commit**

Only if the user explicitly requested commits:

```bash
git add src/api/types.rs src/api/authorship_notes.rs src/git/sync_authorship.rs tests/rest_notes_sync.rs
git commit -m "feat: sync REST authorship notes incrementally"
```

---

## Self-Review

### Spec coverage

- Incremental fetch via `since_change_seq`: covered in Tasks 1, 3, and 5.
- Detect existing-note content changes via `content_hash`: covered in Tasks 2, 3, and 5.
- Push by stable content hash instead of blob OID: covered in Task 4.
- Failure does not advance local watermark: covered in Tasks 2, 3, and 5.
- Old server fallback when `items` is absent: covered in Tasks 1, 3, and 5.
- Existing hook integration unchanged: covered by keeping top-level sync functions and running `fetch_notes`, `internal_machine_commands`, and `push_upstream` tests.
- State file under `.git/ai/rest_notes_sync_state/<repo-key>.json`: covered in Task 2.
- Atomic state write and no watermark rollback: covered in Task 2.
- Remote deletion sync intentionally omitted: no task implements deletion.
- Git protocol notes sync path intentionally unchanged: no task changes the `notes_store = "git"` branch.

### Placeholder scan

The plan avoids open-ended placeholders. Where repository helper names may differ, the plan gives exact fallback code or instructs the implementer to use an already-existing command shape from a named test file.

### Type consistency

The same field names are used throughout: `AuthorshipNotesListItem`, `content_hash`, `change_seq`, `next_change_seq`, `has_more`, `since_change_seq`, `limit`, `RestNotesSyncState`, and `last_change_seq`.
