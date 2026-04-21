use crate::authorship::working_log::CheckpointKind;
use crate::commands::checkpoint_agent::agent_presets::AgentRunResult;
use crate::config::Config;
use crate::error::GitAiError;
use crate::git::repository::Repository;
pub fn pre_commit(repo: &Repository, default_author: String) -> Result<(), GitAiError> {
    let (checkpoint_kind, agent_run_result) = pre_commit_checkpoint_context(repo);

    let result: Result<(usize, usize, usize), GitAiError> = crate::commands::checkpoint::run(
        repo,
        &default_author,
        checkpoint_kind,
        true,
        agent_run_result,
        true, // should skip if NO AI CHECKPOINTS
    );
    result.map(|_| ())?;

    if !Config::get().get_feature_flags().checkpoint_tasks {
        return Ok(());
    }

    let Some(repo_workdir) = repo
        .workdir()
        .ok()
        .map(|path| path.to_string_lossy().to_string())
    else {
        return Ok(());
    };

    let base_commit = match repo.head() {
        Ok(head) => head.target().unwrap_or_else(|_| "initial".to_string()),
        Err(_) => "initial".to_string(),
    };
    let lineage_epoch = crate::checkpoint_tasks::lineage::get_current_epoch(&repo_workdir)?;
    let remaining =
        crate::checkpoint_tasks::recovery::drain_relevant_tasks(repo, &base_commit, lineage_epoch)?;

    if remaining.is_empty() {
        return Ok(());
    }

    let details = remaining
        .iter()
        .map(|task| {
            let mut message = format!("{}:{}", task.task_id, task.state);
            if let Some(error) = task.last_error.as_deref()
                && !error.trim().is_empty()
            {
                message.push_str(&format!(" ({})", error));
            }
            message
        })
        .collect::<Vec<_>>()
        .join(", ");

    Err(GitAiError::Generic(format!(
        "found {} pending checkpoint task(s) relevant to current base commit {}: {}",
        remaining.len(),
        base_commit,
        details
    )))
}

fn pre_commit_checkpoint_context(repo: &Repository) -> (CheckpointKind, Option<AgentRunResult>) {
    let Ok(repo_workdir) = repo
        .workdir()
        .map(|path| path.to_string_lossy().to_string())
    else {
        return (CheckpointKind::Human, None);
    };
    let repo_root = std::path::Path::new(&repo_workdir);

    if let Some((checkpoint_kind, agent_run_result)) =
        crate::commands::checkpoint_agent::bash_tool::checkpoint_context_from_active_bash(
            repo_root,
            &repo_workdir,
        )
    {
        tracing::debug!("pre-commit: using active bash context for AI checkpoint");
        return (checkpoint_kind, agent_run_result);
    }

    tracing::debug!("pre-commit: no active inflight bash agent context, using human checkpoint");
    (CheckpointKind::Human, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint_tasks::store;
    use crate::checkpoint_tasks::types::{CheckpointTaskRecord, CheckpointTaskState};
    use crate::config::Config;
    use crate::feature_flags::FeatureFlags;
    use crate::git::test_utils::TmpRepo;
    use serial_test::serial;
    use std::fs;

    #[test]
    fn test_pre_commit_empty_repo() {
        let test_repo = TmpRepo::new().unwrap();
        let repo = test_repo.gitai_repo();

        // Should handle empty repo gracefully
        let result = pre_commit(repo, "test_author".to_string());
        // May succeed or fail depending on repo state, but shouldn't panic
        let _ = result;
    }

    #[test]
    fn test_pre_commit_with_staged_changes() {
        let test_repo = TmpRepo::new().unwrap();
        let repo = test_repo.gitai_repo();

        // Create and stage a file
        let file_path = test_repo.path().join("test.txt");
        fs::write(&file_path, "test content").unwrap();

        let mut index = test_repo.repo().index().unwrap();
        index.add_path(std::path::Path::new("test.txt")).unwrap();
        index.write().unwrap();

        let result = pre_commit(repo, "test_author".to_string());
        // Should not panic
        let _ = result;
    }

    #[test]
    fn test_pre_commit_no_changes() {
        let test_repo = TmpRepo::new().unwrap();
        let repo = test_repo.gitai_repo();

        // Create initial commit
        let file_path = test_repo.path().join("initial.txt");
        fs::write(&file_path, "initial").unwrap();

        let mut index = test_repo.repo().index().unwrap();
        index.add_path(std::path::Path::new("initial.txt")).unwrap();
        index.write().unwrap();

        let tree_id = index.write_tree().unwrap();
        let tree = test_repo.repo().find_tree(tree_id).unwrap();
        let sig = test_repo.repo().signature().unwrap();

        test_repo
            .repo()
            .commit(Some("HEAD"), &sig, &sig, "Initial commit", &tree, &[])
            .unwrap();

        // Run pre_commit with no staged changes
        let result = pre_commit(repo, "test_author".to_string());
        // Should handle gracefully
        let _ = result;
    }

    #[test]
    fn test_pre_commit_result_mapping() {
        let test_repo = TmpRepo::new().unwrap();
        let repo = test_repo.gitai_repo();

        let result = pre_commit(repo, "author".to_string());

        // Result should be either Ok(()) or Err(GitAiError)
        match result {
            Ok(()) => {
                // Success case
            }
            Err(_) => {
                // Error case is also acceptable
            }
        }
    }

    fn pending_task(repo: &TmpRepo, task_id: &str, base_commit: String) -> CheckpointTaskRecord {
        CheckpointTaskRecord {
            task_id: task_id.to_string(),
            repo_workdir: repo
                .gitai_repo()
                .workdir()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            base_commit,
            lineage_epoch: 0,
            state: CheckpointTaskState::FailedRetryable,
            dedupe_key: format!("dedupe-{}", task_id),
            kind: "human".to_string(),
            author: "claude".to_string(),
            payload_ref: repo
                .path()
                .join(format!("{}.json", task_id))
                .to_string_lossy()
                .to_string(),
            explicit_paths: vec!["test.txt".to_string()],
            is_pre_commit: false,
            captured_at_ms: 1,
            processing_started_at_ms: None,
            applied_at_ms: None,
            completed_at_ms: None,
            obsolete_at_ms: None,
            attempts: 0,
            last_error: Some("pending retry".to_string()),
            next_retry_at_ms: Some(9_000_000_000_000),
        }
    }

    #[test]
    #[serial]
    fn test_pre_commit_blocks_on_relevant_pending_checkpoint_tasks() {
        Config::clear_test_feature_flags();
        Config::set_test_feature_flags(FeatureFlags {
            rewrite_stash: true,
            inter_commit_move: false,
            checkpoint_tasks: true,
            auth_keyring: false,
            async_mode: false,
            git_hooks_enabled: false,
            git_hooks_externally_managed: false,
        });

        let repo = TmpRepo::new_with_base_commit().unwrap().0;
        let base_commit = repo.get_head_commit_sha().unwrap();
        store::create_task(&pending_task(&repo, "pending-task", base_commit.clone())).unwrap();

        let result = pre_commit(repo.gitai_repo(), "author".to_string());

        Config::clear_test_feature_flags();

        let error = result.expect_err("expected pre_commit to block on pending task");
        let message = error.to_string();
        assert!(message.contains("pending checkpoint task"));
        assert!(message.contains("pending-task"));
        assert!(message.contains(&base_commit));
    }

    #[test]
    #[serial]
    fn test_pre_commit_ignores_applied_checkpoint_tasks() {
        Config::clear_test_feature_flags();
        Config::set_test_feature_flags(FeatureFlags {
            rewrite_stash: true,
            inter_commit_move: false,
            checkpoint_tasks: true,
            auth_keyring: false,
            async_mode: false,
            git_hooks_enabled: false,
            git_hooks_externally_managed: false,
        });

        let repo = TmpRepo::new_with_base_commit().unwrap().0;
        let base_commit = repo.get_head_commit_sha().unwrap();
        let mut task = pending_task(&repo, "applied-task", base_commit);
        task.state = CheckpointTaskState::Applied;
        task.applied_at_ms = Some(10);
        task.last_error = None;
        task.next_retry_at_ms = None;
        store::create_task(&task).unwrap();

        let result = pre_commit(repo.gitai_repo(), "author".to_string());

        Config::clear_test_feature_flags();

        assert!(result.is_ok(), "applied tasks should not block pre_commit");
    }

    #[test]
    fn test_pre_commit_checkpoint_context_uses_inflight_bash_agent_context() {
        let test_repo = TmpRepo::new().unwrap();
        let repo = test_repo.gitai_repo();
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("codex-session-simple.jsonl");
        let metadata = std::collections::HashMap::from([(
            "transcript_path".to_string(),
            fixture.to_string_lossy().to_string(),
        )]);
        crate::commands::checkpoint_agent::bash_tool::handle_bash_pre_tool_use_with_context(
            test_repo.path(),
            "session-1",
            "tool-1",
            &crate::authorship::working_log::AgentId {
                tool: "codex".to_string(),
                id: "session-1".to_string(),
                model: "gpt-5.4".to_string(),
            },
            Some(&metadata),
        )
        .unwrap();

        let (checkpoint_kind, agent_run_result) = pre_commit_checkpoint_context(repo);
        assert_eq!(checkpoint_kind, CheckpointKind::AiAgent);
        let agent_run_result = agent_run_result.expect("expected codex agent result");
        assert_eq!(agent_run_result.agent_id.tool, "codex");
        assert_eq!(agent_run_result.agent_id.id, "session-1");
        assert_eq!(
            agent_run_result
                .agent_metadata
                .as_ref()
                .and_then(|m| m.get("transcript_path"))
                .map(String::as_str),
            Some(fixture.to_string_lossy().as_ref())
        );
    }
}
