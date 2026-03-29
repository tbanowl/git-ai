use crate::repos::test_repo::TestRepo;
use crate::rest_test_server::{MockReply, RestTestServer};
use serde_json::json;
use std::collections::HashMap;
use std::fs;

#[test]
fn test_fetch_authorship_notes_rest_mode_returns_not_found_when_remote_empty() {
    let mut repo = TestRepo::new();
    repo.patch_git_ai_config(|patch| {
        patch.notes_store = Some("rest".to_string());
    });

    let routes = HashMap::from([(
        "/api/v1/notes/list".to_string(),
        vec![MockReply {
            status_code: 200,
            body: r#"{"ok":true,"data":{"notes":[]}}"#.to_string(),
        }],
    )]);
    let server = RestTestServer::start(routes);

    let request = json!({
        "remote_name": "https://github.com/example/rest-notes.git"
    })
    .to_string();

    let output = repo
        .git_ai_with_env(
            &["fetch-authorship-notes", "--json", &request],
            &[("GIT_AI_API_BASE_URL", server.base_url())],
        )
        .expect("fetch command should succeed in rest mode");

    let parsed: serde_json::Value =
        serde_json::from_str(output.trim()).expect("fetch output should be JSON");
    assert_eq!(parsed["notes_existence"], "not_found");

    let captured = server.requests();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].method, "POST");
    assert_eq!(captured[0].path, "/api/v1/notes/list");
}

#[test]
fn test_push_authorship_notes_rest_mode_pushes_changed_note_blob() {
    let mut repo = TestRepo::new();
    repo.patch_git_ai_config(|patch| {
        patch.notes_store = Some("rest".to_string());
    });

    fs::write(repo.path().join("sync.txt"), "rest sync note\n").expect("write sync file");
    repo.stage_all_and_commit("rest sync note source")
        .expect("commit should succeed");

    let head_sha = repo
        .git(&["rev-parse", "HEAD"])
        .expect("get head sha")
        .trim()
        .to_string();
    repo.git_og(&[
        "notes",
        "--ref=ai",
        "add",
        "-f",
        "-m",
        "{\"schema_version\":\"authorship/3.0.0\",\"base_commit_sha\":\"x\",\"prompts\":{}}",
        &head_sha,
    ])
    .expect("add deterministic authorship note");

    let notes_list = repo
        .git_og(&["notes", "--ref=ai", "list"])
        .expect("list notes");
    let local_blob_oid = notes_list
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let blob = parts.next()?;
            let commit = parts.next()?;
            if commit == head_sha {
                Some(blob.to_string())
            } else {
                None
            }
        })
        .expect("local note blob oid for HEAD");

    let routes = HashMap::from([
        (
            "/api/v1/notes/list".to_string(),
            vec![MockReply {
                status_code: 200,
                body: format!(
                    "{{\"ok\":true,\"data\":{{\"notes\":[{{\"commit_sha\":\"{}\",\"note_blob_oid\":\"different-blob\"}}]}}}}",
                    head_sha
                ),
            }],
        ),
        (
            "/api/v1/notes/push".to_string(),
            vec![MockReply {
                status_code: 200,
                body: r#"{"ok":true,"data":{"created":1,"updated":0}}"#.to_string(),
            }],
        ),
    ]);
    let server = RestTestServer::start(routes);

    let request = json!({
        "remote_name": "https://github.com/example/rest-notes.git"
    })
    .to_string();
    let output = repo
        .git_ai_with_env(
            &["push-authorship-notes", "--json", &request],
            &[("GIT_AI_API_BASE_URL", server.base_url())],
        )
        .expect("push command should succeed in rest mode");

    let parsed: serde_json::Value =
        serde_json::from_str(output.trim()).expect("push output should be JSON");
    assert_eq!(parsed["ok"], true);

    let captured = server.requests();
    assert_eq!(captured.len(), 2);
    assert_eq!(captured[0].method, "POST");
    assert_eq!(captured[1].method, "POST");
    assert_eq!(captured[0].path, "/api/v1/notes/list");
    assert_eq!(captured[1].path, "/api/v1/notes/push");
    assert!(captured[1].body.contains(&head_sha));
    assert!(captured[1].body.contains(&local_blob_oid));
}
