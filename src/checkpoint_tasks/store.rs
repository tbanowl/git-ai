use crate::authorship::internal_db::InternalDatabase;
use crate::checkpoint_tasks::types::{CheckpointTaskRecord, CheckpointTaskState};
use crate::error::GitAiError;

pub fn create_task(record: &CheckpointTaskRecord) -> Result<(), GitAiError> {
    let db = InternalDatabase::global()?;
    let mut db = db
        .lock()
        .map_err(|_| GitAiError::Generic("internal database lock poisoned".to_string()))?;
    db.upsert_checkpoint_task(record)
}

pub fn get_task(task_id: &str) -> Result<Option<CheckpointTaskRecord>, GitAiError> {
    let db = InternalDatabase::global()?;
    let db = db
        .lock()
        .map_err(|_| GitAiError::Generic("internal database lock poisoned".to_string()))?;
    db.get_checkpoint_task(task_id)
}

pub fn get_task_by_dedupe_key(
    dedupe_key: &str,
) -> Result<Option<CheckpointTaskRecord>, GitAiError> {
    let db = InternalDatabase::global()?;
    let db = db
        .lock()
        .map_err(|_| GitAiError::Generic("internal database lock poisoned".to_string()))?;
    db.get_checkpoint_task_by_dedupe_key(dedupe_key)
}

pub fn list_relevant_tasks(
    repo_workdir: &str,
    base_commit: &str,
    lineage_epoch: u64,
) -> Result<Vec<CheckpointTaskRecord>, GitAiError> {
    let db = InternalDatabase::global()?;
    let db = db
        .lock()
        .map_err(|_| GitAiError::Generic("internal database lock poisoned".to_string()))?;
    db.list_relevant_checkpoint_tasks(repo_workdir, base_commit, lineage_epoch)
}

pub fn list_tasks_by_state(
    state: CheckpointTaskState,
) -> Result<Vec<CheckpointTaskRecord>, GitAiError> {
    let db = InternalDatabase::global()?;
    let db = db
        .lock()
        .map_err(|_| GitAiError::Generic("internal database lock poisoned".to_string()))?;
    db.list_checkpoint_tasks_by_state(state)
}

pub fn list_tasks_for_repo(repo_workdir: &str) -> Result<Vec<CheckpointTaskRecord>, GitAiError> {
    let db = InternalDatabase::global()?;
    let db = db
        .lock()
        .map_err(|_| GitAiError::Generic("internal database lock poisoned".to_string()))?;
    db.list_checkpoint_tasks_for_repo(repo_workdir)
}

pub fn update_task_state(task_id: &str, state: CheckpointTaskState) -> Result<(), GitAiError> {
    let db = InternalDatabase::global()?;
    let mut db = db
        .lock()
        .map_err(|_| GitAiError::Generic("internal database lock poisoned".to_string()))?;
    db.update_checkpoint_task_state(task_id, state)
}

pub fn upsert_task(record: &CheckpointTaskRecord) -> Result<(), GitAiError> {
    create_task(record)
}

pub fn mark_failed_retryable(
    task_id: &str,
    error: &str,
    next_retry_at_ms: Option<u128>,
) -> Result<(), GitAiError> {
    let db = InternalDatabase::global()?;
    let mut db = db
        .lock()
        .map_err(|_| GitAiError::Generic("internal database lock poisoned".to_string()))?;
    db.mark_checkpoint_task_failed_retryable(task_id, error, next_retry_at_ms)
}

pub fn mark_obsolete(
    task_id: &str,
    obsolete_at_ms: u128,
    reason: Option<&str>,
) -> Result<(), GitAiError> {
    let db = InternalDatabase::global()?;
    let mut db = db
        .lock()
        .map_err(|_| GitAiError::Generic("internal database lock poisoned".to_string()))?;
    db.mark_checkpoint_task_obsolete(task_id, obsolete_at_ms, reason)
}

pub fn mark_completed(task_id: &str, completed_at_ms: u128) -> Result<(), GitAiError> {
    let db = InternalDatabase::global()?;
    let mut db = db
        .lock()
        .map_err(|_| GitAiError::Generic("internal database lock poisoned".to_string()))?;
    db.mark_checkpoint_task_completed(task_id, completed_at_ms)
}
