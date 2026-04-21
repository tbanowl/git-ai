use crate::authorship::internal_db::InternalDatabase;
use crate::checkpoint_tasks::store;
use crate::checkpoint_tasks::types::{
    CheckpointLineageState, CheckpointTaskRecord, CheckpointTaskState,
};
use crate::error::GitAiError;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn get_current_epoch(repo_workdir: &str) -> Result<u64, GitAiError> {
    let db = InternalDatabase::global()?;
    let db = db
        .lock()
        .map_err(|_| GitAiError::Generic("internal database lock poisoned".to_string()))?;
    Ok(db
        .get_checkpoint_lineage_state(repo_workdir)?
        .map(|state| state.current_epoch)
        .unwrap_or(0))
}

pub fn set_current_epoch(
    repo_workdir: &str,
    current_epoch: u64,
    updated_at_ms: u128,
) -> Result<(), GitAiError> {
    let db = InternalDatabase::global()?;
    let mut db = db
        .lock()
        .map_err(|_| GitAiError::Generic("internal database lock poisoned".to_string()))?;
    db.set_checkpoint_lineage_state(&CheckpointLineageState {
        repo_workdir: repo_workdir.to_string(),
        current_epoch,
        updated_at_ms,
    })
}

pub fn bump_epoch(repo_workdir: &str, updated_at_ms: u128) -> Result<u64, GitAiError> {
    let next = get_current_epoch(repo_workdir)? + 1;
    set_current_epoch(repo_workdir, next, updated_at_ms)?;
    Ok(next)
}

pub fn is_task_relevant(
    task: &CheckpointTaskRecord,
    repo_workdir: &str,
    base_commit: &str,
    lineage_epoch: u64,
) -> bool {
    task.repo_workdir == repo_workdir
        && task.base_commit == base_commit
        && task.lineage_epoch == lineage_epoch
        && task.state.is_active()
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub fn obsolete_tasks_from_old_epochs(
    repo_workdir: &str,
    current_epoch: u64,
    reason: &str,
) -> Result<Vec<String>, GitAiError> {
    let mut obsoleted = Vec::new();
    for state in [
        CheckpointTaskState::Captured,
        CheckpointTaskState::Ready,
        CheckpointTaskState::Applying,
        CheckpointTaskState::FailedRetryable,
    ] {
        for task in store::list_tasks_by_state(state)? {
            if task.repo_workdir != repo_workdir || task.lineage_epoch >= current_epoch {
                continue;
            }

            store::mark_obsolete(&task.task_id, now_ms(), Some(reason))?;
            obsoleted.push(task.task_id);
        }
    }
    Ok(obsoleted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;

    #[test]
    fn test_is_task_relevant_only_for_matching_active_task() {
        let task = CheckpointTaskRecord {
            task_id: "task-1".to_string(),
            repo_workdir: "/repo".to_string(),
            base_commit: "abc123".to_string(),
            lineage_epoch: 2,
            state: CheckpointTaskState::Ready,
            dedupe_key: "dedupe-1".to_string(),
            kind: "ai_agent".to_string(),
            author: "claude".to_string(),
            payload_ref: "payloads/task-1.json".to_string(),
            explicit_paths: vec![],
            is_pre_commit: false,
            captured_at_ms: 10,
            processing_started_at_ms: None,
            applied_at_ms: None,
            completed_at_ms: None,
            obsolete_at_ms: None,
            attempts: 0,
            last_error: None,
            next_retry_at_ms: None,
        };

        assert!(is_task_relevant(&task, "/repo", "abc123", 2));
        assert!(!is_task_relevant(&task, "/repo", "def456", 2));
        assert!(!is_task_relevant(&task, "/other", "abc123", 2));

        let applied = CheckpointTaskRecord {
            state: CheckpointTaskState::Applied,
            ..task.clone()
        };
        assert!(!is_task_relevant(&applied, "/repo", "abc123", 2));
    }

    #[test]
    #[serial]
    fn test_obsolete_tasks_from_old_epochs_marks_only_older_active_tasks() {
        let repo = crate::git::test_utils::TmpRepo::new_with_base_commit()
            .unwrap()
            .0;
        let repo_workdir = repo
            .gitai_repo()
            .workdir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let base_commit = repo.get_head_commit_sha().unwrap();

        let old_ready = CheckpointTaskRecord {
            task_id: "old-ready".to_string(),
            repo_workdir: repo_workdir.clone(),
            base_commit: base_commit.clone(),
            lineage_epoch: 0,
            state: CheckpointTaskState::Ready,
            dedupe_key: "dedupe-old-ready".to_string(),
            kind: "human".to_string(),
            author: "author".to_string(),
            payload_ref: repo
                .path()
                .join("old-ready.json")
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
            last_error: None,
            next_retry_at_ms: None,
        };
        let current_ready = CheckpointTaskRecord {
            task_id: "current-ready".to_string(),
            lineage_epoch: 2,
            dedupe_key: "dedupe-current-ready".to_string(),
            payload_ref: repo
                .path()
                .join("current-ready.json")
                .to_string_lossy()
                .to_string(),
            ..old_ready.clone()
        };
        let old_applied = CheckpointTaskRecord {
            task_id: "old-applied".to_string(),
            lineage_epoch: 0,
            state: CheckpointTaskState::Applied,
            dedupe_key: "dedupe-old-applied".to_string(),
            payload_ref: repo
                .path()
                .join("old-applied.json")
                .to_string_lossy()
                .to_string(),
            applied_at_ms: Some(10),
            ..old_ready.clone()
        };

        store::create_task(&old_ready).unwrap();
        store::create_task(&current_ready).unwrap();
        store::create_task(&old_applied).unwrap();

        let obsoleted = obsolete_tasks_from_old_epochs(&repo_workdir, 2, "reset").unwrap();
        assert_eq!(obsoleted, vec!["old-ready".to_string()]);

        let old_ready_after = store::get_task("old-ready").unwrap().unwrap();
        assert_eq!(old_ready_after.state, CheckpointTaskState::Obsolete);

        let current_ready_after = store::get_task("current-ready").unwrap().unwrap();
        assert_eq!(current_ready_after.state, CheckpointTaskState::Ready);

        let old_applied_after = store::get_task("old-applied").unwrap().unwrap();
        assert_eq!(old_applied_after.state, CheckpointTaskState::Applied);
    }
}
