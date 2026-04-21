use crate::checkpoint_tasks::lineage;
use crate::checkpoint_tasks::recovery;
use crate::checkpoint_tasks::store;
use crate::checkpoint_tasks::types::CheckpointTaskState;
use crate::error::GitAiError;
use crate::git::find_repository;

pub fn handle_checkpoint_recover(args: &[String]) {
    let mut json_output = false;
    for arg in args {
        if arg == "--json" {
            json_output = true;
        }
    }

    if let Err(e) = run_checkpoint_recover(json_output) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run_checkpoint_recover(json: bool) -> Result<(), GitAiError> {
    let repo = find_repository(&[])?;
    let repo_workdir = repo.workdir()?.to_string_lossy().to_string();
    let base_commit = repo.head()?.target()?;
    let lineage_epoch = lineage::get_current_epoch(&repo_workdir)?;

    let before = store::list_tasks_for_repo(&repo_workdir)?;
    let before_pending: Vec<_> = before
        .iter()
        .filter(|t| t.state.is_active())
        .map(|t| t.task_id.clone())
        .collect();

    recovery::recover_stale_applying_tasks(
        &repo_workdir,
        recovery::DEFAULT_STALE_APPLYING_TIMEOUT_MS,
    )?;

    let remaining = recovery::drain_relevant_tasks(&repo, &base_commit, lineage_epoch)?;

    let after = store::list_tasks_for_repo(&repo_workdir)?;
    let recovered: Vec<_> = before_pending
        .iter()
        .filter(|id| {
            after
                .iter()
                .find(|t| &t.task_id == *id)
                .map(|t| !t.state.is_active())
                .unwrap_or(false)
        })
        .cloned()
        .collect();

    let still_pending: Vec<_> = remaining
        .iter()
        .map(|t| (t.task_id.clone(), t.state.to_string(), t.last_error.clone()))
        .collect();

    let obsolete_count = after
        .iter()
        .filter(|t| t.state == CheckpointTaskState::Obsolete)
        .count();

    if json {
        let output = serde_json::json!({
            "recovered": recovered,
            "still_pending": still_pending.iter().map(|(id, state, err)| serde_json::json!({
                "task_id": id,
                "state": state,
                "last_error": err,
            })).collect::<Vec<_>>(),
            "obsolete_count": obsolete_count,
        });
        println!("{}", output);
        return Ok(());
    }

    println!("Checkpoint recovery complete.");
    println!("  Recovered:     {}", recovered.len());
    println!("  Still pending: {}", still_pending.len());
    println!("  Obsolete:      {}", obsolete_count);

    if !still_pending.is_empty() {
        println!();
        println!("Pending tasks:");
        for (id, state, err) in &still_pending {
            let err_str = err.as_deref().unwrap_or("<none>");
            println!("  {}  state={}  error={}", id, state, err_str);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::checkpoint_tasks::store;
    use crate::checkpoint_tasks::types::{CheckpointTaskRecord, CheckpointTaskState};
    use crate::git::test_utils::TmpRepo;
    use serial_test::serial;

    fn applied_task(repo: &TmpRepo, task_id: &str, base_commit: String) -> CheckpointTaskRecord {
        CheckpointTaskRecord {
            task_id: task_id.to_string(),
            repo_workdir: repo
                .gitai_repo()
                .workdir()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            base_commit,
            lineage_epoch: lineage::get_current_epoch(
                repo.gitai_repo()
                    .workdir()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref(),
            )
            .unwrap(),
            state: CheckpointTaskState::Applied,
            dedupe_key: format!("dedupe-{}", task_id),
            kind: "human".to_string(),
            author: "author".to_string(),
            payload_ref: repo
                .path()
                .join(format!("{}.json", task_id))
                .to_string_lossy()
                .to_string(),
            explicit_paths: vec!["test.txt".to_string()],
            is_pre_commit: false,
            captured_at_ms: 1,
            processing_started_at_ms: None,
            applied_at_ms: Some(10),
            completed_at_ms: None,
            obsolete_at_ms: None,
            attempts: 0,
            last_error: None,
            next_retry_at_ms: None,
        }
    }

    #[test]
    #[serial]
    fn test_run_checkpoint_recover_with_no_pending_tasks_succeeds() {
        let repo = TmpRepo::new_with_base_commit().unwrap().0;
        let repo_workdir = repo
            .gitai_repo()
            .workdir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let base_commit = repo.get_head_commit_sha().unwrap();
        store::create_task(&applied_task(&repo, "applied-task", base_commit)).unwrap();

        let result = run_checkpoint_recover(false);
        assert!(
            result.is_ok(),
            "recovery with no pending tasks should succeed"
        );

        let tasks = store::list_tasks_for_repo(&repo_workdir).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].state, CheckpointTaskState::Applied);
    }
}
