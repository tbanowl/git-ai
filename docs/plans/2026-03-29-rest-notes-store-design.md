# REST Notes Store Design

## Problem

Some git remote hosting platforms (e.g., Alibaba Codeup) do not support push/fetch of `git notes`. This prevents git-ai from synchronizing authorship notes across clones via the standard `git push/fetch refs/notes/ai` mechanism.

## Solution

Add a configurable REST API backend for remote notes synchronization. Local git notes operations remain unchanged — only the remote sync channel (push/fetch) is replaced by REST API calls when configured.

## Architecture

```
                          notes_store: "git" (default)
                         ┌──────────────────────────────┐
                         │  git fetch/push refs/notes/ai │
                         └──────────────────────────────┘
                        /
sync_authorship.rs ────<
                        \
                         ┌──────────────────────────────┐
                         │  REST API (api_base_url)      │
                         └──────────────────────────────┘
                          notes_store: "rest"
```

**Key principle:** Local git notes read/write (add, show, search, merge) is completely untouched. Only `fetch_authorship_notes()` and `push_authorship_notes()` in `sync_authorship.rs` gain a REST code path.

---

## Server Side (Python + Flask + SQLAlchemy)

### Tech Stack

- **Language:** Python 3.10+
- **Framework:** Flask
- **ORM:** SQLAlchemy
- **Database:** SQLite (initial), Oracle (future migration)
- **ID Generation:** xid (time-ordered globally unique IDs)
- **Deployment:** SaaS (hosted) or self-hosted (private deployment)

### Data Model

```sql
CREATE TABLE authorship_notes (
    id                  TEXT PRIMARY KEY,       -- xid
    repo_url            TEXT NOT NULL,          -- repository remote URL
    branch              TEXT NOT NULL,          -- branch name
    commit_sha          TEXT NOT NULL,          -- current commit SHA (40 chars)
    original_commit_sha TEXT,                   -- original commit SHA before rebase/cherry-pick
    author_name         TEXT NOT NULL,          -- committer name
    author_email        TEXT NOT NULL,          -- committer email
    note_content        TEXT NOT NULL,          -- AuthorshipLog raw content
    created_at          INTEGER NOT NULL,       -- unix timestamp
    updated_at          INTEGER NOT NULL,       -- unix timestamp
    UNIQUE(repo_url, commit_sha)
);

CREATE INDEX idx_authorship_notes_repo_url ON authorship_notes(repo_url);
CREATE INDEX idx_authorship_notes_repo_commit ON authorship_notes(repo_url, commit_sha);
```

### REST API Endpoints

All endpoints require authentication via existing mechanisms:
- `Authorization: Bearer {token}` (OAuth)
- or `X-API-Key: {key}` + `X-Author-Identity: {name <email>}`

#### PUT /api/v1/notes

Create or update a single note.

**Request:**
```json
{
  "repo_url": "https://codeup.aliyun.com/org/repo.git",
  "branch": "main",
  "commit_sha": "abc123def456...",
  "original_commit_sha": null,
  "author_name": "John Doe",
  "author_email": "john@example.com",
  "content": "<authorship log content>"
}
```

**Response (200):**
```json
{
  "ok": true,
  "data": { "id": "ctg3h1e..." }
}
```

#### POST /api/v1/notes/get

Retrieve a single note by repo_url + commit_sha.

**Request:**
```json
{
  "repo_url": "https://codeup.aliyun.com/org/repo.git",
  "commit_sha": "abc123def456..."
}
```

**Response (200):**
```json
{
  "ok": true,
  "data": {
    "id": "ctg3h1e...",
    "commit_sha": "abc123def456...",
    "branch": "main",
    "author_name": "John Doe",
    "author_email": "john@example.com",
    "content": "<authorship log content>",
    "created_at": 1711670400,
    "updated_at": 1711670400
  }
}
```

**Response (404):**
```json
{
  "ok": false,
  "error": "note not found"
}
```

#### POST /api/v1/notes/batch

Batch retrieve notes for multiple commit SHAs.

**Request:**
```json
{
  "repo_url": "https://codeup.aliyun.com/org/repo.git",
  "commit_shas": ["abc123...", "def456...", "789ghi..."]
}
```

**Response (200):**
```json
{
  "ok": true,
  "data": {
    "notes": [
      {
        "commit_sha": "abc123...",
        "content": "<authorship log content>"
      },
      {
        "commit_sha": "def456...",
        "content": "<authorship log content>"
      }
    ],
    "missing": ["789ghi..."]
  }
}
```

#### POST /api/v1/notes/push

Batch push (create/update) notes.

**Request:**
```json
{
  "repo_url": "https://codeup.aliyun.com/org/repo.git",
  "notes": [
    {
      "branch": "main",
      "commit_sha": "abc123...",
      "original_commit_sha": null,
      "author_name": "John Doe",
      "author_email": "john@example.com",
      "content": "<authorship log content>"
    }
  ]
}
```

**Response (200):**
```json
{
  "ok": true,
  "data": { "created": 3, "updated": 1 }
}
```

#### POST /api/v1/notes/list

List all commit SHAs that have notes for a given repo.

**Request:**
```json
{
  "repo_url": "https://codeup.aliyun.com/org/repo.git"
}
```

**Response (200):**
```json
{
  "ok": true,
  "data": {
    "commit_shas": ["abc123...", "def456...", ...]
  }
}
```

#### POST /api/v1/notes/search

Full-text search in notes content.

**Request:**
```json
{
  "repo_url": "https://codeup.aliyun.com/org/repo.git",
  "pattern": "cursor"
}
```

**Response (200):**
```json
{
  "ok": true,
  "data": {
    "commit_shas": ["abc123...", "def456..."]
  }
}
```

### Error Response Format

All errors follow:
```json
{
  "ok": false,
  "error": "<human-readable message>"
}
```

HTTP status codes: 400 (bad request), 401 (unauthorized), 404 (not found), 500 (server error).

---

## Client Side (Rust changes)

### Configuration Change

In `config.rs`, add one field:

```rust
// New field in Config
notes_store: Option<String>  // "git" (default) | "rest"
```

Environment variable override: `GIT_AI_NOTES_STORE`

The REST API URL reuses the existing `api_base_url` configuration (default: `https://usegitai.com`).

### New File: src/api/notes_api.rs (~100 lines)

Two public functions that encapsulate REST API calls:

```rust
/// Fetch notes from REST API and write to local git notes.
/// Returns NotesExistence::Found if any notes were fetched.
pub fn rest_fetch_notes(
    repo: &Repository,
    api: &ApiClient,
    repo_url: &str,
) -> Result<NotesExistence, GitAiError>

/// Read local git notes and push to REST API.
pub fn rest_push_notes(
    repo: &Repository,
    api: &ApiClient,
    repo_url: &str,
) -> Result<(), GitAiError>
```

**rest_fetch_notes flow:**
1. Call `POST /api/v1/notes/list` to get remote commit SHAs
2. Compare against local notes (using `refs::note_blob_oids_for_commits()`)
3. Call `POST /api/v1/notes/batch` for missing SHAs
4. Write fetched notes to local git notes via `refs::notes_add_batch()`

**rest_push_notes flow:**
1. List local commits with notes (using git log + refs::note_blob_oids_for_commits)
2. Call `POST /api/v1/notes/list` to get remote commit SHAs
3. Diff to find local-only notes
4. Read note content for each (via `refs::show_authorship_note()`)
5. Collect branch, author_name, author_email from git commit metadata
6. Call `POST /api/v1/notes/push` to upload

### Modified File: src/git/sync_authorship.rs

Add REST branch to existing functions:

```rust
pub fn fetch_authorship_notes(
    repository: &Repository,
    remote_name: &str,
) -> Result<NotesExistence, GitAiError> {
    let config = Config::get();
    if config.notes_store() == "rest" {
        let api = ApiClient::new();
        let repo_url = repository.remote_url(remote_name)?;
        return rest_fetch_notes(repository, &api, &repo_url);
    }
    // ... existing git fetch logic unchanged ...
}

pub fn push_authorship_notes(
    repository: &Repository,
    remote_name: &str,
) -> Result<(), GitAiError> {
    let config = Config::get();
    if config.notes_store() == "rest" {
        let api = ApiClient::new();
        let repo_url = repository.remote_url(remote_name)?;
        return rest_push_notes(repository, &api, &repo_url);
    }
    // ... existing git push logic unchanged ...
}
```

### Files Changed Summary

| File | Change |
|------|--------|
| `config.rs` | Add `notes_store` field + getter + env override |
| `sync_authorship.rs` | Add REST branch at top of fetch/push functions |
| `api/notes_api.rs` | **New file** — REST fetch/push implementations |
| `api/mod.rs` | Add `pub mod notes_api;` |

Total client-side change: ~120 lines of new code, ~10 lines of modifications.

---

## Server Project Structure

Independent repository, structured as:

```
git-ai-notes-server/
├── app/
│   ├── __init__.py          # Flask app factory
│   ├── config.py            # Configuration (DB URL, etc.)
│   ├── models.py            # SQLAlchemy models
│   ├── routes/
│   │   ├── __init__.py
│   │   └── notes.py         # Notes API endpoints
│   ├── auth.py              # Authentication middleware
│   └── errors.py            # Error handlers
├── migrations/               # Alembic migrations
├── tests/
│   └── test_notes.py
├── requirements.txt
└── run.py                   # Entry point
```

---

## Authentication

Reuses the existing git-ai authentication mechanism:
- OAuth token via `Authorization: Bearer {token}`
- API key via `X-API-Key: {key}` + `X-Author-Identity: {name <email>}`

Server validates tokens against the same auth backend as the main git-ai API.

---

## Non-Goals

- No changes to local git notes operations (add, show, search, merge)
- No changes to hook dispatch logic (push_hooks, fetch_hooks, clone_hooks)
- No dual-sync (git + REST simultaneously)
- No offline queue / retry for REST mode (may add later)
