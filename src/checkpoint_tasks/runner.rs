use crate::checkpoint_tasks::evidence;
use crate::checkpoint_tasks::store;
use crate::checkpoint_tasks::types::{
    CheckpointAppliedEvidence, CheckpointTaskRecord, CheckpointTaskState,
};
use crate::commands::checkpoint::{
    execute_durable_checkpoint_payload, load_durable_checkpoint_task_payload,
};
use crate::error::GitAiError;
use crate::git::repository::Repository;
use std::time::{SystemTime, UNIX_EPOCH};

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

fn mark_applying(mut task: CheckpointTaskRecord) -> Result<CheckpointTaskRecord, GitAiError> {
    task.state = CheckpointTaskState::Applying;
    task.processing_started_at_ms = Some(now_ms());
    store::upsert_task(&task)?;
    Ok(task)
}

fn mark_applied(mut task: CheckpointTaskRecord, applied_at_ms: u128) -> Result<(), GitAiError> {
    task.state = CheckpointTaskState::Applied;
    task.applied_at_ms = Some(applied_at_ms);
    task.processing_started_at_ms = None;
    task.last_error = None;
    task.next_retry_at_ms = None;
    store::upsert_task(&task)
}

pub fn run_task(repo: &Repository, task_id: &str) -> Result<(usize, usize, usize), GitAiError> {
    let task = store::get_task(task_id)?
        .ok_or_else(|| GitAiError::Generic(format!("checkpoint task not found: {}", task_id)))?;

    if evidence::has_applied_evidence(task_id)? {
        mark_applied(task.clone(), task.applied_at_ms.unwrap_or_else(now_ms))?;
        return Ok((0, task.explicit_paths.len(), 0));
    }

    if matches!(
        task.state,
        CheckpointTaskState::Applied | CheckpointTaskState::Completed
    ) {
        return Ok((0, task.explicit_paths.len(), 0));
    }

    let task = mark_applying(task)?;
    let payload = match load_durable_checkpoint_task_payload(&task) {
        Ok(payload) => payload,
        Err(error) => {
            let now = now_ms();
            store::mark_failed_retryable(
                &task.task_id,
                &error.to_string(),
                Some(calculate_next_retry_at_ms(task.attempts + 1, now)),
            )?;
            return Err(error);
        }
    };
    let result = match execute_durable_checkpoint_payload(repo, &payload) {
        Ok(result) => result,
        Err(error) => {
            let now = now_ms();
            store::mark_failed_retryable(
                &task.task_id,
                &error.to_string(),
                Some(calculate_next_retry_at_ms(task.attempts + 1, now)),
            )?;
            return Err(error);
        }
    };
    let applied_at_ms = now_ms();

    evidence::record_applied_evidence(&CheckpointAppliedEvidence {
        task_id: task.task_id.clone(),
        dedupe_key: task.dedupe_key.clone(),
        base_commit: task.base_commit.clone(),
        applied_at_ms,
        apply_result_hash: None,
    })?;

    mark_applied(task, applied_at_ms)?;
    Ok(result)
}
