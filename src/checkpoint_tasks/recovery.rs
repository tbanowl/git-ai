use crate::checkpoint_tasks::evidence;
use crate::checkpoint_tasks::runner;
use crate::checkpoint_tasks::store;
use crate::checkpoint_tasks::types::CheckpointTaskState;
use crate::error::GitAiError;
use crate::git::repository::Repository;
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_STALE_APPLYING_TIMEOUT_MS: u128 = 120_000;

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn calculate_next_retry_at_ms(next_attempt: u32, now_ms: u128) -> u128 {
    let delay_ms = match next_attempt {
        1 => 5 * 60 * 1000,
        2 => 30 * 60 * 1000,
        3 => 2 * 60 * 60 * 1000,
        4 => 6 * 60 * 60 * 1000,
        5 => 12 * 60 * 60 * 1000,
        _ => 24 * 60 * 60 * 1000,
    } as u128;
    now_ms + delay_ms
}

fn recover_stale_applying_tasks_at(
    repo_workdir: &str,
    stale_after_ms: u128,
    now_ms: u128,
) -> Result<Vec<String>, GitAiError> {
    let mut recovered = Vec::new();
    for task in store::list_tasks_by_state(CheckpointTaskState::Applying)? {
        if task.repo_workdir != repo_workdir {
            continue;
        }

        let Some(started_at) = task.processing_started_at_ms else {
            continue;
        };

        if now_ms.saturating_sub(started_at) < stale_after_ms {
            continue;
        }

        if let Some(existing_evidence) = evidence::get_applied_evidence(&task.task_id)? {
            let mut updated = task.clone();
            updated.state = CheckpointTaskState::Applied;
            updated.processing_started_at_ms = None;
            updated.applied_at_ms = Some(existing_evidence.applied_at_ms);
            updated.last_error = None;
            updated.next_retry_at_ms = None;
            store::upsert_task(&updated)?;
        } else {
            store::mark_failed_retryable(
                &task.task_id,
                "recovered stale checkpoint task stuck in applying",
                Some(calculate_next_retry_at_ms(task.attempts + 1, now_ms)),
            )?;
        }
        recovered.push(task.task_id);
    }
    Ok(recovered)
}

pub fn recover_stale_applying_tasks(
    repo_workdir: &str,
    stale_after_ms: u128,
) -> Result<Vec<String>, GitAiError> {
    recover_stale_applying_tasks_at(repo_workdir, stale_after_ms, now_ms())
}

fn drain_relevant_tasks_at(
    repo: &Repository,
    base_commit: &str,
    lineage_epoch: u64,
    now_ms: u128,
) -> Result<Vec<crate::checkpoint_tasks::types::CheckpointTaskRecord>, GitAiError> {
    let repo_workdir = repo.workdir()?.to_string_lossy().to_string();
    recover_stale_applying_tasks_at(&repo_workdir, DEFAULT_STALE_APPLYING_TIMEOUT_MS, now_ms)?;

    let tasks = store::list_relevant_tasks(&repo_workdir, base_commit, lineage_epoch)?;
    for task in tasks {
        match task.state {
            CheckpointTaskState::Captured => {
                store::update_task_state(&task.task_id, CheckpointTaskState::Ready)?;
                let _ = runner::run_task(repo, &task.task_id);
            }
            CheckpointTaskState::Ready => {
                let _ = runner::run_task(repo, &task.task_id);
            }
            CheckpointTaskState::FailedRetryable => {
                if task.next_retry_at_ms.unwrap_or(0) <= now_ms {
                    let _ = runner::run_task(repo, &task.task_id);
                }
            }
            CheckpointTaskState::Applying
            | CheckpointTaskState::Applied
            | CheckpointTaskState::Completed
            | CheckpointTaskState::Obsolete => {}
        }
    }

    store::list_relevant_tasks(&repo_workdir, base_commit, lineage_epoch)
}

pub fn drain_relevant_tasks(
    repo: &Repository,
    base_commit: &str,
    lineage_epoch: u64,
) -> Result<Vec<crate::checkpoint_tasks::types::CheckpointTaskRecord>, GitAiError> {
    drain_relevant_tasks_at(repo, base_commit, lineage_epoch, now_ms())
}

pub fn retry_due_failed_tasks(
    repo: &Repository,
    base_commit: &str,
    lineage_epoch: u64,
) -> Result<Vec<crate::checkpoint_tasks::types::CheckpointTaskRecord>, GitAiError> {
    let repo_workdir = repo.workdir()?.to_string_lossy().to_string();
    let now = now_ms();
    for task in store::list_relevant_tasks(&repo_workdir, base_commit, lineage_epoch)? {
        if task.state == CheckpointTaskState::FailedRetryable
            && task.next_retry_at_ms.unwrap_or(0) <= now
        {
            let _ = runner::run_task(repo, &task.task_id);
        }
    }
    store::list_relevant_tasks(&repo_workdir, base_commit, lineage_epoch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorship::transcript::AiTranscript;
    use crate::authorship::working_log::{AgentId, CheckpointKind};
    use crate::checkpoint_tasks::types::{
        CheckpointAppliedEvidence, CheckpointTaskRecord, CheckpointTaskState,
    };
    use crate::commands::checkpoint::DurableCheckpointTaskPayload;
    use crate::git::test_utils::TmpRepo;
    use serial_test::serial;
    use std::collections::HashMap;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn write_payload(
        repo_workdir: &str,
        task_id: &str,
        payload: &DurableCheckpointTaskPayload,
    ) -> String {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::path::Path::new(repo_workdir)
            .join(".git")
            .join("ai")
            .join(format!("recovery-test-{}-{}.json", task_id, unique));
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, serde_json::to_vec(payload).unwrap()).unwrap();
        path.to_string_lossy().to_string()
    }

    fn sample_task(
        task_id: &str,
        repo_workdir: String,
        base_commit: String,
        payload_ref: String,
    ) -> CheckpointTaskRecord {
        CheckpointTaskRecord {
            task_id: task_id.to_string(),
            repo_workdir,
            base_commit,
            lineage_epoch: 0,
            state: CheckpointTaskState::Applying,
            dedupe_key: format!("dedupe-{}", task_id),
            kind: "ai_agent".to_string(),
            author: "claude".to_string(),
            payload_ref,
            explicit_paths: vec!["test.txt".to_string()],
            is_pre_commit: false,
            captured_at_ms: 1,
            processing_started_at_ms: Some(1),
            applied_at_ms: None,
            completed_at_ms: None,
            obsolete_at_ms: None,
            attempts: 0,
            last_error: None,
            next_retry_at_ms: None,
        }
    }

    #[test]
    #[serial]
    fn test_recover_stale_applying_with_evidence_marks_task_applied() {
        let (repo, _file, _) = TmpRepo::new_with_base_commit().unwrap();
        let repo_workdir = repo
            .gitai_repo()
            .workdir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let payload = DurableCheckpointTaskPayload {
            author: "claude".to_string(),
            kind: CheckpointKind::AiAgent,
            is_pre_commit: false,
            base_commit: repo.get_head_commit_sha().unwrap(),
            captured_at_ms: 1,
            files: vec!["test.txt".to_string()],
            dirty_files: HashMap::new(),
            explicit_paths: vec!["test.txt".to_string()],
            agent_run_result: Some(
                crate::commands::checkpoint_agent::agent_presets::AgentRunResult {
                    agent_id: AgentId {
                        tool: "codex".to_string(),
                        id: "session".to_string(),
                        model: "gpt-5".to_string(),
                    },
                    agent_metadata: None,
                    checkpoint_kind: CheckpointKind::AiAgent,
                    transcript: Some(AiTranscript { messages: vec![] }),
                    repo_working_dir: Some(repo_workdir.clone()),
                    edited_filepaths: Some(vec!["test.txt".to_string()]),
                    will_edit_filepaths: None,
                    dirty_files: None,
                    captured_checkpoint_id: None,
                },
            ),
        };
        let payload_ref = write_payload(&repo_workdir, "stale-evidence", &payload);
        let task = sample_task(
            "stale-evidence",
            repo_workdir.clone(),
            repo.get_head_commit_sha().unwrap(),
            payload_ref,
        );
        store::create_task(&task).unwrap();
        evidence::record_applied_evidence(&CheckpointAppliedEvidence {
            task_id: task.task_id.clone(),
            dedupe_key: task.dedupe_key.clone(),
            base_commit: task.base_commit.clone(),
            applied_at_ms: 42,
            apply_result_hash: None,
        })
        .unwrap();

        let recovered = recover_stale_applying_tasks_at(&repo_workdir, 10, 1_000).unwrap();
        assert_eq!(recovered, vec![task.task_id.clone()]);

        let updated = store::get_task(&task.task_id).unwrap().unwrap();
        assert_eq!(updated.state, CheckpointTaskState::Applied);
        assert_eq!(updated.applied_at_ms, Some(42));
        assert_eq!(updated.processing_started_at_ms, None);
    }

    #[test]
    #[serial]
    fn test_recover_stale_applying_without_evidence_marks_task_retryable() {
        let (repo, _file, _) = TmpRepo::new_with_base_commit().unwrap();
        let repo_workdir = repo
            .gitai_repo()
            .workdir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let payload = DurableCheckpointTaskPayload {
            author: "claude".to_string(),
            kind: CheckpointKind::AiAgent,
            is_pre_commit: false,
            base_commit: repo.get_head_commit_sha().unwrap(),
            captured_at_ms: 1,
            files: vec!["test.txt".to_string()],
            dirty_files: HashMap::new(),
            explicit_paths: vec!["test.txt".to_string()],
            agent_run_result: None,
        };
        let payload_ref = write_payload(&repo_workdir, "stale-no-evidence", &payload);
        let task = sample_task(
            "stale-no-evidence",
            repo_workdir.clone(),
            repo.get_head_commit_sha().unwrap(),
            payload_ref,
        );
        store::create_task(&task).unwrap();

        let recovered = recover_stale_applying_tasks_at(&repo_workdir, 10, 1_000).unwrap();
        assert_eq!(recovered, vec![task.task_id.clone()]);

        let updated = store::get_task(&task.task_id).unwrap().unwrap();
        assert_eq!(updated.state, CheckpointTaskState::FailedRetryable);
        assert_eq!(updated.attempts, 1);
        assert!(updated.next_retry_at_ms.unwrap() > 1_000);
    }

    #[test]
    #[serial]
    fn test_drain_relevant_tasks_runs_due_retryable_task() {
        let (repo, mut file, _) = TmpRepo::new_with_base_commit().unwrap();
        file.append("line from recovery\n").unwrap();

        let repo_workdir = repo
            .gitai_repo()
            .workdir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let base_commit = repo.get_head_commit_sha().unwrap();
        let payload = DurableCheckpointTaskPayload {
            author: "claude".to_string(),
            kind: CheckpointKind::Human,
            is_pre_commit: false,
            base_commit: base_commit.clone(),
            captured_at_ms: 1,
            files: vec![file.filename().to_string()],
            dirty_files: HashMap::new(),
            explicit_paths: vec![file.filename().to_string()],
            agent_run_result: None,
        };
        let payload_ref = write_payload(&repo_workdir, "retryable", &payload);
        let task = CheckpointTaskRecord {
            state: CheckpointTaskState::FailedRetryable,
            next_retry_at_ms: Some(0),
            processing_started_at_ms: None,
            payload_ref,
            repo_workdir: repo_workdir.clone(),
            base_commit: base_commit.clone(),
            explicit_paths: vec![file.filename().to_string()],
            task_id: "retryable".to_string(),
            dedupe_key: "dedupe-retryable".to_string(),
            kind: "human".to_string(),
            author: "claude".to_string(),
            lineage_epoch: crate::checkpoint_tasks::lineage::get_current_epoch(&repo_workdir)
                .unwrap(),
            captured_at_ms: 1,
            applied_at_ms: None,
            completed_at_ms: None,
            obsolete_at_ms: None,
            attempts: 0,
            last_error: Some("previous failure".to_string()),
            is_pre_commit: false,
        };
        store::create_task(&task).unwrap();

        let remaining = drain_relevant_tasks_at(
            repo.gitai_repo(),
            &base_commit,
            crate::checkpoint_tasks::lineage::get_current_epoch(&repo_workdir).unwrap(),
            1_000,
        )
        .unwrap();

        assert!(remaining.is_empty());
        let updated = store::get_task(&task.task_id).unwrap().unwrap();
        assert_eq!(
            updated.state,
            CheckpointTaskState::Applied,
            "expected retryable task to be applied, last_error={:?}",
            updated.last_error
        );
        assert!(evidence::has_applied_evidence(&task.task_id).unwrap());
    }
}
