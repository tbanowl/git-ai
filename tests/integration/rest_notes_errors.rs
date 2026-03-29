use crate::repos::test_repo::TestRepo;
use crate::rest_test_server::{MockReply, RestTestServer};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_rest_fetch_empty_list_maps_to_not_found() {
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
        .expect("fetch should succeed");

    let parsed: serde_json::Value = serde_json::from_str(output.trim()).expect("valid json");
    assert_eq!(parsed["notes_existence"], "not_found");
}

#[test]
fn test_rest_fetch_http_404_is_hard_error() {
    let mut repo = TestRepo::new();
    repo.patch_git_ai_config(|patch| {
        patch.notes_store = Some("rest".to_string());
    });

    let routes = HashMap::from([(
        "/api/v1/notes/list".to_string(),
        vec![MockReply {
            status_code: 404,
            body: r#"{"error":"missing notes endpoint"}"#.to_string(),
        }],
    )]);
    let server = RestTestServer::start(routes);

    let request = json!({
        "remote_name": "https://github.com/example/rest-notes.git"
    })
    .to_string();
    let err = repo
        .git_ai_with_env(
            &["fetch-authorship-notes", "--json", &request],
            &[("GIT_AI_API_BASE_URL", server.base_url())],
        )
        .expect_err("fetch should fail on endpoint 404");

    let parsed: serde_json::Value = serde_json::from_str(err.trim()).expect("valid json error");
    let message = parsed["error"]
        .as_str()
        .expect("json error should include message");
    assert!(message.contains("status 404"));
}
