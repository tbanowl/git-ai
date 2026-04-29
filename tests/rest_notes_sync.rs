#[macro_use]
#[path = "integration/repos/mod.rs"]
mod repos;

use git_ai::git::sync_authorship::{read_rest_notes_sync_state, sha256_note_content};
use repos::test_repo::{DaemonTestScope, GitTestMode, TestRepo};
use serde_json::{Value, json};
use serial_test::serial;
use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

const REPO_URL: &str = "https://example.com/owner/repo";

#[derive(Debug)]
struct RecordedRequest {
    path: String,
    body: Value,
}

struct RestNotesMockServer {
    base_url: String,
    requests: mpsc::Receiver<RecordedRequest>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

enum MockResponse {
    Json(Value),
    Status { code: u16, body: Value },
}

impl From<Value> for MockResponse {
    fn from(value: Value) -> Self {
        Self::Json(value)
    }
}

impl RestNotesMockServer {
    fn start(responses: Vec<Value>) -> Self {
        Self::start_with_responses(responses.into_iter().map(MockResponse::Json).collect())
    }

    fn start_with_responses(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock REST notes server");
        let addr = listener.local_addr().expect("read mock listener addr");
        let (tx, rx) = mpsc::channel();
        let responses = Arc::new(Mutex::new(VecDeque::<MockResponse>::from(responses)));
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = Arc::clone(&stop);
        let responses_thread = Arc::clone(&responses);
        let (ready_tx, ready_rx) = mpsc::channel();

        let thread = thread::spawn(move || {
            ready_tx.send(()).expect("signal mock server readiness");
            while !stop_thread.load(Ordering::SeqCst) {
                if responses_thread
                    .lock()
                    .expect("lock mock responses")
                    .is_empty()
                {
                    break;
                }
                match listener.accept() {
                    Ok((stream, _)) if stop_thread.load(Ordering::SeqCst) => {
                        drop(stream);
                        break;
                    }
                    Ok((stream, _)) => handle_http_connection(stream, &tx, &responses_thread),
                    Err(error) => panic!("mock REST notes accept failed: {}", error),
                }
            }
        });
        ready_rx.recv().expect("mock server should become ready");

        Self {
            base_url: format!("http://{}", addr),
            requests: rx,
            stop,
            thread: Some(thread),
        }
    }

    fn base_url(&self) -> &str {
        &self.base_url
    }

    fn recv_request(&self) -> RecordedRequest {
        self.requests
            .recv_timeout(Duration::from_secs(15))
            .expect("timed out waiting for REST notes request")
    }

    fn assert_no_request(&self) {
        match self.requests.recv_timeout(Duration::from_millis(250)) {
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {}
            Ok(request) => panic!("unexpected REST notes request: {:?}", request),
        }
    }
}

impl Drop for RestNotesMockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.base_url.trim_start_matches("http://"));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn handle_http_connection(
    mut stream: TcpStream,
    tx: &mpsc::Sender<RecordedRequest>,
    responses: &Arc<Mutex<VecDeque<MockResponse>>>,
) {
    let mut pending = Vec::new();
    let Some((path, body)) = read_http_request(&mut stream, &mut pending) else {
        return;
    };

    let body_json: Value = serde_json::from_slice(&body).expect("request should be JSON");
    tx.send(RecordedRequest {
        path,
        body: body_json,
    })
    .expect("record mock request");

    let response = responses
        .lock()
        .expect("lock mock responses")
        .pop_front()
        .unwrap_or_else(|| MockResponse::Status {
            code: 500,
            body: json!({"error": "mock response exhausted"}),
        });
    let write_result = match response {
        MockResponse::Json(value) => {
            let response_body = serde_json::to_vec(&value).expect("serialize mock response");
            write_http_response(&mut stream, 200, "OK", &response_body)
        }
        MockResponse::Status { code, body } => {
            let response_body = serde_json::to_vec(&body).expect("serialize mock response");
            write_http_response(&mut stream, code, "Error", &response_body)
        }
    };
    if write_result.is_err() {
        return;
    }
}

fn read_http_request(stream: &mut TcpStream, pending: &mut Vec<u8>) -> Option<(String, Vec<u8>)> {
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("set mock read timeout");
    let header_end = loop {
        if let Some(end) = find_header_end(pending) {
            break end;
        }
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        pending.extend_from_slice(&chunk[..read]);
    };

    let headers = String::from_utf8_lossy(&pending[..header_end]);
    let request_line = headers.lines().next()?;
    let path = request_line.split_whitespace().nth(1)?.to_string();
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.split_once(':').and_then(|(name, value)| {
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().ok())
                    .flatten()
            })
        })
        .unwrap_or(0);

    let request_end = header_end + content_length;
    while pending.len() < request_end {
        let mut chunk = [0u8; 4096];
        let read = stream.read(&mut chunk).ok()?;
        if read == 0 {
            return None;
        }
        pending.extend_from_slice(&chunk[..read]);
    }

    let body = pending[header_end..request_end].to_vec();
    pending.drain(..request_end);

    Some((path, body))
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|idx| idx + 4)
}

fn write_http_response(
    stream: &mut TcpStream,
    code: u16,
    reason: &str,
    body: &[u8],
) -> std::io::Result<()> {
    let headers = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        code,
        reason,
        body.len(),
    );
    stream.write_all(headers.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn test_repo() -> TestRepo {
    let mut repo =
        TestRepo::new_with_mode_and_daemon_scope(GitTestMode::Wrapper, DaemonTestScope::Dedicated);
    repo.patch_git_ai_config(|patch| {
        patch.notes_store = Some("rest".to_string());
        patch.telemetry_oss_disabled = Some(true);
    });
    repo.git_og(&["remote", "add", "origin", REPO_URL])
        .expect("add origin remote");
    repo
}

fn create_commit_without_note(repo: &TestRepo, filename: &str, content: &str) -> String {
    fs::write(repo.path().join(filename), content).expect("write test file");
    repo.git_og(&["add", filename]).expect("stage file");
    repo.git_og(&["commit", "-m", &format!("add {}", filename)])
        .expect("commit file");
    repo.git_og(&["rev-parse", "HEAD"])
        .expect("resolve HEAD")
        .trim()
        .to_string()
}

fn add_local_note(repo: &TestRepo, commit_sha: &str, content: &str) {
    repo.git_og(&["notes", "--ref=ai", "add", "-f", "-m", content, commit_sha])
        .expect("add local authorship note");
}

fn git_common_dir(repo: &TestRepo) -> PathBuf {
    let raw = repo
        .git_og(&["rev-parse", "--git-common-dir"])
        .expect("resolve git common dir");
    let path = PathBuf::from(raw.trim());
    if path.is_absolute() {
        path
    } else {
        repo.path().join(path)
    }
}

fn fetch_notes(repo: &TestRepo, api_base_url: &str) -> Result<String, String> {
    repo.git_ai_with_env(
        &["fetch-notes", "origin", "--json"],
        &[
            ("GIT_AI_API_BASE_URL", api_base_url),
            ("GIT_AI_API_KEY", "test-api-key"),
            ("GIT_AI_NOTES_STORE", "rest"),
        ],
    )
}

fn push_notes(repo: &TestRepo, api_base_url: &str) -> Result<String, String> {
    repo.git_ai_with_env(
        &[
            "push-authorship-notes",
            "--json",
            r#"{"remote_name":"origin"}"#,
        ],
        &[
            ("GIT_AI_API_BASE_URL", api_base_url),
            ("GIT_AI_API_KEY", "test-api-key"),
            ("GIT_AI_NOTES_STORE", "rest"),
        ],
    )
}

fn list_response(commit_sha: &str, note_content: &str, change_seq: i64) -> Value {
    json!({
        "ok": true,
        "data": {
            "commit_shas": [commit_sha],
            "note_blob_oids": null,
            "items": [{
                "commit_sha": commit_sha,
                "content_hash": format!("sha256:{}", sha256_note_content(note_content)),
                "change_seq": change_seq,
                "updated_at": 1234
            }],
            "next_change_seq": change_seq,
            "has_more": false
        }
    })
}

fn paged_list_response(
    commit_sha: &str,
    note_content: &str,
    item_change_seq: i64,
    next_change_seq: i64,
    has_more: bool,
) -> Value {
    json!({
        "ok": true,
        "data": {
            "commit_shas": [commit_sha],
            "note_blob_oids": null,
            "items": [{
                "commit_sha": commit_sha,
                "content_hash": format!("sha256:{}", sha256_note_content(note_content)),
                "change_seq": item_change_seq,
                "updated_at": 1234
            }],
            "next_change_seq": next_change_seq,
            "has_more": has_more
        }
    })
}

fn legacy_list_response(commit_sha: &str) -> Value {
    json!({
        "ok": true,
        "data": {
            "commit_shas": [commit_sha],
            "note_blob_oids": null
        }
    })
}

fn batch_response(commit_sha: &str, note_content: &str, hash_content: &str) -> Value {
    json!({
        "ok": true,
        "data": {
            "notes": [{
                "commit_sha": commit_sha,
                "content": note_content,
                "note_blob_oid": null,
                "content_hash": sha256_note_content(hash_content),
                "change_seq": 10
            }],
            "missing": []
        }
    })
}

fn push_response(created: usize, updated: usize) -> Value {
    json!({
        "ok": true,
        "data": {
            "created": created,
            "updated": updated,
            "unchanged": 0
        }
    })
}

#[test]
#[serial]
fn rest_fetch_multi_page_advances_watermark_to_final_page() {
    let repo = test_repo();
    let first_commit = create_commit_without_note(&repo, "page-one.txt", "page one target\n");
    let second_commit = create_commit_without_note(&repo, "page-two.txt", "page two target\n");
    let first_note = "remote page one note\n";
    let second_note = "remote page two note\n";
    let first_server = RestNotesMockServer::start(vec![
        paged_list_response(&first_commit, first_note, 1, 1, true),
        batch_response(&first_commit, first_note, first_note),
        paged_list_response(&second_commit, second_note, 2, 2, false),
        batch_response(&second_commit, second_note, second_note),
        json!({
            "ok": true,
            "data": {
                "commit_shas": [],
                "note_blob_oids": null,
                "items": [],
                "next_change_seq": 2,
                "has_more": false
            }
        }),
    ]);

    fetch_notes(&repo, first_server.base_url()).expect("multi-page REST fetch should succeed");

    let first_list = first_server.recv_request();
    assert_eq!(first_list.path, "/worker/authorship_notes/list");
    assert_eq!(first_list.body["since_change_seq"], 0);
    assert_eq!(first_list.body["limit"], 1000);
    let first_batch = first_server.recv_request();
    assert_eq!(first_batch.path, "/worker/authorship_notes/batch");
    assert_eq!(
        first_batch.body["commit_shas"],
        json!([first_commit.clone()])
    );
    let second_list = first_server.recv_request();
    assert_eq!(second_list.path, "/worker/authorship_notes/list");
    assert_eq!(second_list.body["since_change_seq"], 1);
    let second_batch = first_server.recv_request();
    assert_eq!(second_batch.path, "/worker/authorship_notes/batch");
    assert_eq!(
        second_batch.body["commit_shas"],
        json!([second_commit.clone()])
    );

    assert_eq!(
        repo.read_authorship_note(&first_commit)
            .as_deref()
            .map(str::trim_end),
        Some(first_note.trim_end())
    );
    assert_eq!(
        repo.read_authorship_note(&second_commit)
            .as_deref()
            .map(str::trim_end),
        Some(second_note.trim_end())
    );
    let state = read_rest_notes_sync_state(&git_common_dir(&repo), REPO_URL).unwrap();
    assert_eq!(state.last_change_seq, 2);

    fetch_notes(&repo, first_server.base_url()).expect("second REST fetch should succeed");
    let next_list = first_server.recv_request();
    assert_eq!(next_list.path, "/worker/authorship_notes/list");
    assert_eq!(next_list.body["since_change_seq"], 2);
}

#[test]
#[serial]
fn rest_fetch_second_page_failure_preserves_old_watermark() {
    let repo = test_repo();
    let commit_sha = create_commit_without_note(&repo, "first-page.txt", "first page target\n");
    let remote_note = "first page remote note\n";
    let failing_server = RestNotesMockServer::start_with_responses(vec![
        paged_list_response(&commit_sha, remote_note, 1, 1, true).into(),
        batch_response(&commit_sha, remote_note, remote_note).into(),
        MockResponse::Status {
            code: 500,
            body: json!({"error": "second page failed"}),
        },
        json!({
            "ok": true,
            "data": {
                "commit_shas": [],
                "note_blob_oids": null,
                "items": [],
                "next_change_seq": 0,
                "has_more": false
            }
        })
        .into(),
    ]);

    let err = fetch_notes(&repo, failing_server.base_url())
        .expect_err("second page server error should fail fetch");
    assert!(
        err.contains("HTTP request failed") || err.contains("500"),
        "unexpected fetch error: {}",
        err
    );

    let first_list = failing_server.recv_request();
    assert_eq!(first_list.path, "/worker/authorship_notes/list");
    assert_eq!(first_list.body["since_change_seq"], 0);
    let first_batch = failing_server.recv_request();
    assert_eq!(first_batch.path, "/worker/authorship_notes/batch");
    let second_list = failing_server.recv_request();
    assert_eq!(second_list.path, "/worker/authorship_notes/list");
    assert_eq!(second_list.body["since_change_seq"], 1);
    let state = read_rest_notes_sync_state(&git_common_dir(&repo), REPO_URL).unwrap();
    assert_eq!(state.last_change_seq, 0);

    fetch_notes(&repo, failing_server.base_url()).expect("retry fetch should use old watermark");
    let retry_list = failing_server.recv_request();
    assert_eq!(retry_list.path, "/worker/authorship_notes/list");
    assert_eq!(retry_list.body["since_change_seq"], 0);
}

#[test]
#[serial]
fn rest_fetch_legacy_list_fallback_restores_note_without_writing_watermark() {
    let repo = test_repo();
    let commit_sha = create_commit_without_note(&repo, "legacy-fetch.txt", "legacy target\n");
    let remote_note = "legacy remote note\n";
    let legacy_server = RestNotesMockServer::start(vec![
        legacy_list_response(&commit_sha),
        legacy_list_response(&commit_sha),
        batch_response(&commit_sha, remote_note, remote_note),
        json!({
            "ok": true,
            "data": {
                "commit_shas": [],
                "note_blob_oids": null,
                "items": [],
                "next_change_seq": 0,
                "has_more": false
            }
        }),
    ]);

    fetch_notes(&repo, legacy_server.base_url()).expect("legacy fallback fetch should succeed");

    let incremental_list = legacy_server.recv_request();
    assert_eq!(incremental_list.path, "/worker/authorship_notes/list");
    assert_eq!(incremental_list.body["since_change_seq"], 0);
    assert_eq!(incremental_list.body["limit"], 1000);
    let legacy_list = legacy_server.recv_request();
    assert_eq!(legacy_list.path, "/worker/authorship_notes/list");
    assert!(legacy_list.body.get("since_change_seq").is_none());
    assert!(legacy_list.body.get("limit").is_none());
    let batch = legacy_server.recv_request();
    assert_eq!(batch.path, "/worker/authorship_notes/batch");
    assert_eq!(batch.body["commit_shas"], json!([commit_sha.clone()]));
    assert_eq!(
        repo.read_authorship_note(&commit_sha)
            .as_deref()
            .map(str::trim_end),
        Some(remote_note.trim_end())
    );
    let state = read_rest_notes_sync_state(&git_common_dir(&repo), REPO_URL).unwrap();
    assert_eq!(state.last_change_seq, 0);

    fetch_notes(&repo, legacy_server.base_url()).expect("next incremental fetch should succeed");
    let next_list = legacy_server.recv_request();
    assert_eq!(next_list.path, "/worker/authorship_notes/list");
    assert_eq!(next_list.body["since_change_seq"], 0);
}

#[test]
#[serial]
fn rest_fetch_remote_hash_mismatch_overwrites_existing_local_note() {
    let repo = test_repo();
    let commit_sha = create_commit_without_note(&repo, "overwrite.txt", "overwrite target\n");
    let stale_note = "stale local note";
    let remote_note = "fresh remote note";
    add_local_note(&repo, &commit_sha, stale_note);
    let server = RestNotesMockServer::start(vec![
        list_response(&commit_sha, remote_note, 15),
        batch_response(&commit_sha, remote_note, remote_note),
    ]);

    fetch_notes(&repo, server.base_url()).expect("REST fetch should overwrite stale local note");

    let list = server.recv_request();
    assert_eq!(list.path, "/worker/authorship_notes/list");
    let batch = server.recv_request();
    assert_eq!(batch.path, "/worker/authorship_notes/batch");
    assert_eq!(batch.body["commit_shas"], json!([commit_sha.clone()]));
    assert_eq!(
        repo.read_authorship_note(&commit_sha)
            .as_deref()
            .map(str::trim_end),
        Some(remote_note)
    );
    let state = read_rest_notes_sync_state(&git_common_dir(&repo), REPO_URL).unwrap();
    assert_eq!(state.last_change_seq, 15);
}

#[test]
#[serial]
fn rest_fetch_downloads_missing_note_and_advances_state_from_zero() {
    let repo = test_repo();
    let commit_sha = create_commit_without_note(&repo, "missing.txt", "missing note target\n");
    let remote_note = "remote authorship note\n";
    let server = RestNotesMockServer::start(vec![
        list_response(&commit_sha, remote_note, 10),
        batch_response(&commit_sha, remote_note, remote_note),
    ]);

    fetch_notes(&repo, server.base_url()).expect("REST fetch should succeed");

    let list = server.recv_request();
    assert_eq!(list.path, "/worker/authorship_notes/list");
    assert_eq!(list.body["repo_url"], REPO_URL);
    assert_eq!(list.body["since_change_seq"], 0);
    assert_eq!(list.body["limit"], 1000);

    let batch = server.recv_request();
    assert_eq!(batch.path, "/worker/authorship_notes/batch");
    assert_eq!(batch.body["commit_shas"], json!([commit_sha.clone()]));

    assert_eq!(
        repo.read_authorship_note(&commit_sha)
            .as_deref()
            .map(str::trim_end),
        Some(remote_note.trim_end())
    );
    let state = read_rest_notes_sync_state(&git_common_dir(&repo), REPO_URL).unwrap();
    assert_eq!(state.last_change_seq, 10);
}

#[test]
#[serial]
fn rest_fetch_skips_batch_when_local_hash_matches() {
    let repo = test_repo();
    let commit_sha = create_commit_without_note(&repo, "present.txt", "local note target\n");
    let local_note = "already present note";
    add_local_note(&repo, &commit_sha, local_note);
    let server = RestNotesMockServer::start(vec![list_response(&commit_sha, local_note, 11)]);

    fetch_notes(&repo, server.base_url()).expect("REST fetch should succeed");

    let list = server.recv_request();
    assert_eq!(list.path, "/worker/authorship_notes/list");
    server.assert_no_request();
    assert_eq!(
        repo.read_authorship_note(&commit_sha)
            .as_deref()
            .map(str::trim_end),
        Some(local_note)
    );
    let state = read_rest_notes_sync_state(&git_common_dir(&repo), REPO_URL).unwrap();
    assert_eq!(state.last_change_seq, 11);
}

#[test]
#[serial]
fn rest_fetch_batch_hash_mismatch_errors_without_writing_note_or_advancing_state() {
    let repo = test_repo();
    let commit_sha = create_commit_without_note(&repo, "mismatch.txt", "hash mismatch target\n");
    let expected_note = "expected remote note";
    let wrong_note = "wrong remote note";
    let server = RestNotesMockServer::start(vec![
        list_response(&commit_sha, expected_note, 12),
        batch_response(&commit_sha, wrong_note, expected_note),
    ]);

    let err = fetch_notes(&repo, server.base_url()).expect_err("REST fetch should fail");
    assert!(
        err.contains("content hash"),
        "unexpected fetch error: {}",
        err
    );

    let list = server.recv_request();
    assert_eq!(list.path, "/worker/authorship_notes/list");
    let batch = server.recv_request();
    assert_eq!(batch.path, "/worker/authorship_notes/batch");
    assert_eq!(repo.read_authorship_note(&commit_sha), None);
    let state = read_rest_notes_sync_state(&git_common_dir(&repo), REPO_URL).unwrap();
    assert_eq!(state.last_change_seq, 0);
}

#[test]
#[serial]
fn rest_push_skips_push_when_remote_hash_matches() {
    let repo = test_repo();
    let commit_sha = create_commit_without_note(&repo, "matching.txt", "matching note target\n");
    let local_note = "local authorship note";
    add_local_note(&repo, &commit_sha, local_note);
    let server = RestNotesMockServer::start(vec![list_response(&commit_sha, local_note, 13)]);

    push_notes(&repo, server.base_url()).expect("REST push should succeed");

    let list = server.recv_request();
    assert_eq!(list.path, "/worker/authorship_notes/list");
    assert_eq!(list.body["repo_url"], REPO_URL);
    assert_eq!(list.body["since_change_seq"], 0);
    assert_eq!(list.body["limit"], 1000);
    server.assert_no_request();
    let state = read_rest_notes_sync_state(&git_common_dir(&repo), REPO_URL).unwrap();
    assert_eq!(state.last_change_seq, 0);
}

#[test]
#[serial]
fn rest_push_pushes_one_note_when_remote_hash_differs() {
    let repo = test_repo();
    let commit_sha = create_commit_without_note(&repo, "changed.txt", "changed note target\n");
    let local_note = "new local authorship note";
    let remote_note = "old remote authorship note";
    add_local_note(&repo, &commit_sha, local_note);
    let server = RestNotesMockServer::start(vec![
        list_response(&commit_sha, remote_note, 14),
        push_response(0, 1),
    ]);

    push_notes(&repo, server.base_url()).expect("REST push should succeed");

    let list = server.recv_request();
    assert_eq!(list.path, "/worker/authorship_notes/list");
    let push = server.recv_request();
    assert_eq!(push.path, "/worker/authorship_notes/push");
    assert_eq!(push.body["repo_url"], REPO_URL);
    assert_eq!(push.body["notes"].as_array().unwrap().len(), 1);
    assert_eq!(push.body["notes"][0]["commit_sha"], commit_sha);
    assert_eq!(push.body["notes"][0]["content"], local_note);
    let state = read_rest_notes_sync_state(&git_common_dir(&repo), REPO_URL).unwrap();
    assert_eq!(state.last_change_seq, 0);
}

#[test]
#[serial]
fn rest_push_pushes_one_note_when_remote_commit_missing() {
    let repo = test_repo();
    let commit_sha =
        create_commit_without_note(&repo, "missing-remote.txt", "missing remote note target\n");
    let local_note = "local note missing remotely";
    add_local_note(&repo, &commit_sha, local_note);
    let server = RestNotesMockServer::start(vec![
        json!({
            "ok": true,
            "data": {
                "commit_shas": [],
                "note_blob_oids": null,
                "items": [],
                "next_change_seq": 0,
                "has_more": false
            }
        }),
        push_response(1, 0),
    ]);

    push_notes(&repo, server.base_url()).expect("REST push should succeed");

    let list = server.recv_request();
    assert_eq!(list.path, "/worker/authorship_notes/list");
    let push = server.recv_request();
    assert_eq!(push.path, "/worker/authorship_notes/push");
    assert_eq!(push.body["notes"].as_array().unwrap().len(), 1);
    assert_eq!(push.body["notes"][0]["commit_sha"], commit_sha);
    assert_eq!(push.body["notes"][0]["content"], local_note);
    let state = read_rest_notes_sync_state(&git_common_dir(&repo), REPO_URL).unwrap();
    assert_eq!(state.last_change_seq, 0);
}

#[test]
#[serial]
fn rest_push_old_server_list_without_items_skips_existing_commit() {
    let repo = test_repo();
    let commit_sha =
        create_commit_without_note(&repo, "old-server.txt", "old server note target\n");
    let local_note = "local note already known to old server";
    add_local_note(&repo, &commit_sha, local_note);
    let server = RestNotesMockServer::start(vec![json!({
        "ok": true,
        "data": {
            "commit_shas": [commit_sha],
            "note_blob_oids": null
        }
    })]);

    push_notes(&repo, server.base_url()).expect("REST push should succeed");

    let list = server.recv_request();
    assert_eq!(list.path, "/worker/authorship_notes/list");
    assert_eq!(list.body["repo_url"], REPO_URL);
    assert_eq!(list.body["since_change_seq"], 0);
    assert_eq!(list.body["limit"], 1000);
    server.assert_no_request();
    let state = read_rest_notes_sync_state(&git_common_dir(&repo), REPO_URL).unwrap();
    assert_eq!(state.last_change_seq, 0);
}
