use crate::authorship::internal_db::InternalDatabase;
use crate::checkpoint_tasks::types::CheckpointAppliedEvidence;
use crate::error::GitAiError;

pub fn record_applied_evidence(evidence: &CheckpointAppliedEvidence) -> Result<(), GitAiError> {
    let db = InternalDatabase::global()?;
    let mut db = db
        .lock()
        .map_err(|_| GitAiError::Generic("internal database lock poisoned".to_string()))?;
    db.record_checkpoint_applied_evidence(evidence)
}

pub fn get_applied_evidence(
    task_id: &str,
) -> Result<Option<CheckpointAppliedEvidence>, GitAiError> {
    let db = InternalDatabase::global()?;
    let db = db
        .lock()
        .map_err(|_| GitAiError::Generic("internal database lock poisoned".to_string()))?;
    db.get_checkpoint_applied_evidence(task_id)
}

pub fn has_applied_evidence(task_id: &str) -> Result<bool, GitAiError> {
    Ok(get_applied_evidence(task_id)?.is_some())
}
