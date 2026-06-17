use crate::error::GitAiError;
use crate::git::repo_state::is_valid_git_oid;
use crate::utils::LockFile;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PendingRebasePickStatus {
    Pending,
    Consumed,
    Aborted,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRebasePick {
    pub source_commit: String,
    pub expected_parent: String,
    pub original_head: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onto_head: Option<String>,
    pub operation: String,
    pub status: PendingRebasePickStatus,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumed_by: Option<String>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn lock_path_for(path: &Path) -> std::path::PathBuf {
    path.with_extension("json.lock")
}

fn with_pending_rebase_pick_lock<T>(
    path: &Path,
    f: impl FnOnce() -> Result<T, GitAiError>,
) -> Result<T, GitAiError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock_path = lock_path_for(path);
    let _lock = LockFile::try_acquire(&lock_path).ok_or_else(|| {
        GitAiError::Generic("timed out waiting for pending rebase pick lock".to_string())
    })?;
    f()
}

pub fn pending_rebase_pick(
    source_commit: String,
    expected_parent: String,
    original_head: String,
    onto_head: Option<String>,
    operation: impl Into<String>,
) -> PendingRebasePick {
    PendingRebasePick {
        source_commit,
        expected_parent,
        original_head,
        onto_head,
        operation: operation.into(),
        status: PendingRebasePickStatus::Pending,
        created_at_ms: now_ms(),
        consumed_by: None,
    }
}

pub fn read_pending_rebase_pick(path: &Path) -> Result<Option<PendingRebasePick>, GitAiError> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path)?;
    if raw.trim().is_empty() {
        return Ok(None);
    }
    let pick = serde_json::from_str(&raw)?;
    Ok(Some(pick))
}

pub fn write_pending_rebase_pick(path: &Path, pick: &PendingRebasePick) -> Result<(), GitAiError> {
    with_pending_rebase_pick_lock(path, || write_pending_rebase_pick_unlocked(path, pick))
}

pub fn take_pending_rebase_pick_for_commit(
    path: &Path,
    pre_head: &str,
    new_commit: &str,
) -> Result<Option<PendingRebasePick>, GitAiError> {
    with_pending_rebase_pick_lock(path, || {
        let Some(mut pick) = read_pending_rebase_pick(path)? else {
            return Ok(None);
        };
        if pick.status != PendingRebasePickStatus::Pending {
            return Ok(None);
        }
        if pick.expected_parent != pre_head {
            return Ok(None);
        }

        let consumed = pick.clone();
        pick.status = PendingRebasePickStatus::Consumed;
        pick.consumed_by = Some(new_commit.to_string());
        write_pending_rebase_pick_unlocked(path, &pick)?;
        Ok(Some(consumed))
    })
}

pub fn mark_pending_rebase_pick_aborted(path: &Path) -> Result<(), GitAiError> {
    mark_pending_rebase_pick(path, PendingRebasePickStatus::Aborted)
}

pub fn mark_pending_rebase_pick_skipped(path: &Path) -> Result<(), GitAiError> {
    mark_pending_rebase_pick(path, PendingRebasePickStatus::Skipped)
}

pub fn rebase_in_progress(git_dir: &Path) -> bool {
    git_dir.join("rebase-merge").exists() || git_dir.join("rebase-apply").exists()
}

pub fn stopped_rebase_source_commit(git_dir: &Path) -> Option<String> {
    read_first_valid_oid(&[
        git_dir.join("REBASE_HEAD"),
        git_dir.join("rebase-merge").join("stopped-sha"),
        git_dir.join("rebase-apply").join("stopped-sha"),
    ])
}

fn mark_pending_rebase_pick(
    path: &Path,
    status: PendingRebasePickStatus,
) -> Result<(), GitAiError> {
    with_pending_rebase_pick_lock(path, || {
        if let Some(mut pick) = read_pending_rebase_pick(path)?
            && pick.status == PendingRebasePickStatus::Pending
        {
            pick.status = status;
            write_pending_rebase_pick_unlocked(path, &pick)?;
        }
        Ok(())
    })
}

fn write_pending_rebase_pick_unlocked(
    path: &Path,
    pick: &PendingRebasePick,
) -> Result<(), GitAiError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let raw = serde_json::to_vec_pretty(pick)?;
    fs::write(&tmp, raw)?;
    fs::rename(tmp, path)?;
    Ok(())
}

fn read_first_valid_oid(paths: &[std::path::PathBuf]) -> Option<String> {
    paths.iter().find_map(|path| {
        fs::read_to_string(path)
            .ok()
            .and_then(|raw| {
                raw.lines()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                    .map(str::to_string)
            })
            .filter(|oid| is_valid_git_oid(oid) && !oid.chars().all(|ch| ch == '0'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn pick() -> PendingRebasePick {
        PendingRebasePick {
            source_commit: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            expected_parent: "cccccccccccccccccccccccccccccccccccccccc".to_string(),
            original_head: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string(),
            onto_head: Some("cccccccccccccccccccccccccccccccccccccccc".to_string()),
            operation: "pull_rebase_conflict".to_string(),
            status: PendingRebasePickStatus::Pending,
            created_at_ms: 123,
            consumed_by: None,
        }
    }

    #[test]
    fn create_and_read_pending_pick_round_trips() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("pending_rebase_pick.json");

        write_pending_rebase_pick(&path, &pick()).unwrap();
        let read = read_pending_rebase_pick(&path).unwrap().unwrap();

        assert_eq!(read, pick());
    }

    #[test]
    fn take_pending_pick_consumes_only_matching_parent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("pending_rebase_pick.json");
        write_pending_rebase_pick(&path, &pick()).unwrap();

        let missed = take_pending_rebase_pick_for_commit(
            &path,
            "dddddddddddddddddddddddddddddddddddddddd",
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        )
        .unwrap();
        assert!(missed.is_none());
        assert_eq!(
            read_pending_rebase_pick(&path).unwrap().unwrap().status,
            PendingRebasePickStatus::Pending
        );

        let consumed = take_pending_rebase_pick_for_commit(
            &path,
            "cccccccccccccccccccccccccccccccccccccccc",
            "dddddddddddddddddddddddddddddddddddddddd",
        )
        .unwrap()
        .unwrap();
        assert_eq!(consumed.source_commit, pick().source_commit);

        let stored = read_pending_rebase_pick(&path).unwrap().unwrap();
        assert_eq!(stored.status, PendingRebasePickStatus::Consumed);
        assert_eq!(
            stored.consumed_by.as_deref(),
            Some("dddddddddddddddddddddddddddddddddddddddd")
        );

        let second_take = take_pending_rebase_pick_for_commit(
            &path,
            "cccccccccccccccccccccccccccccccccccccccc",
            "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee",
        )
        .unwrap();
        assert!(second_take.is_none());
    }

    #[test]
    fn mark_pending_pick_aborted_and_skipped_are_persistent() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("pending_rebase_pick.json");
        write_pending_rebase_pick(&path, &pick()).unwrap();

        mark_pending_rebase_pick_aborted(&path).unwrap();
        assert_eq!(
            read_pending_rebase_pick(&path).unwrap().unwrap().status,
            PendingRebasePickStatus::Aborted
        );

        write_pending_rebase_pick(&path, &pick()).unwrap();
        mark_pending_rebase_pick_skipped(&path).unwrap();
        assert_eq!(
            read_pending_rebase_pick(&path).unwrap().unwrap().status,
            PendingRebasePickStatus::Skipped
        );
    }
}
