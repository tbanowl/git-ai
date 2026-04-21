use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointTaskState {
    Captured,
    Ready,
    Applying,
    Applied,
    Completed,
    FailedRetryable,
    Obsolete,
}

impl CheckpointTaskState {
    pub fn as_str(self) -> &'static str {
        match self {
            CheckpointTaskState::Captured => "captured",
            CheckpointTaskState::Ready => "ready",
            CheckpointTaskState::Applying => "applying",
            CheckpointTaskState::Applied => "applied",
            CheckpointTaskState::Completed => "completed",
            CheckpointTaskState::FailedRetryable => "failed_retryable",
            CheckpointTaskState::Obsolete => "obsolete",
        }
    }

    pub fn is_active(self) -> bool {
        !matches!(
            self,
            CheckpointTaskState::Applied
                | CheckpointTaskState::Completed
                | CheckpointTaskState::Obsolete
        )
    }
}

impl fmt::Display for CheckpointTaskState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CheckpointTaskState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "captured" => Ok(CheckpointTaskState::Captured),
            "ready" => Ok(CheckpointTaskState::Ready),
            "applying" => Ok(CheckpointTaskState::Applying),
            "applied" => Ok(CheckpointTaskState::Applied),
            "completed" => Ok(CheckpointTaskState::Completed),
            "failed_retryable" => Ok(CheckpointTaskState::FailedRetryable),
            "obsolete" => Ok(CheckpointTaskState::Obsolete),
            other => Err(format!("unknown checkpoint task state: {}", other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointTaskRecord {
    pub task_id: String,
    pub repo_workdir: String,
    pub base_commit: String,
    pub lineage_epoch: u64,
    pub state: CheckpointTaskState,
    pub dedupe_key: String,
    pub kind: String,
    pub author: String,
    pub payload_ref: String,
    pub explicit_paths: Vec<String>,
    pub is_pre_commit: bool,
    pub captured_at_ms: u128,
    pub processing_started_at_ms: Option<u128>,
    pub applied_at_ms: Option<u128>,
    pub completed_at_ms: Option<u128>,
    pub obsolete_at_ms: Option<u128>,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub next_retry_at_ms: Option<u128>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointAppliedEvidence {
    pub task_id: String,
    pub dedupe_key: String,
    pub base_commit: String,
    pub applied_at_ms: u128,
    pub apply_result_hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointLineageState {
    pub repo_workdir: String,
    pub current_epoch: u64,
    pub updated_at_ms: u128,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_checkpoint_task_state_serializes_as_snake_case() {
        let serialized = serde_json::to_string(&CheckpointTaskState::FailedRetryable).unwrap();
        assert_eq!(serialized, "\"failed_retryable\"");
    }

    #[test]
    fn test_checkpoint_task_state_round_trips_from_str() {
        let parsed: CheckpointTaskState = "obsolete".parse().unwrap();
        assert_eq!(parsed, CheckpointTaskState::Obsolete);
        assert_eq!(CheckpointTaskState::Applying.to_string(), "applying");
    }

    #[test]
    fn test_checkpoint_task_record_round_trips() {
        let record = CheckpointTaskRecord {
            task_id: "task-1".to_string(),
            repo_workdir: "/repo".to_string(),
            base_commit: "abc123".to_string(),
            lineage_epoch: 42,
            state: CheckpointTaskState::Ready,
            dedupe_key: "dedupe-1".to_string(),
            kind: "ai_agent".to_string(),
            author: "claude".to_string(),
            payload_ref: "payloads/task-1.json".to_string(),
            explicit_paths: vec!["src/lib.rs".to_string(), "src/main.rs".to_string()],
            is_pre_commit: true,
            captured_at_ms: 100,
            processing_started_at_ms: Some(200),
            applied_at_ms: Some(300),
            completed_at_ms: None,
            obsolete_at_ms: None,
            attempts: 2,
            last_error: Some("temporary failure".to_string()),
            next_retry_at_ms: Some(400),
        };

        let serialized = serde_json::to_string(&record).unwrap();
        let deserialized: CheckpointTaskRecord = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, record);
    }

    #[test]
    fn test_checkpoint_applied_evidence_round_trips() {
        let evidence = CheckpointAppliedEvidence {
            task_id: "task-1".to_string(),
            dedupe_key: "dedupe-1".to_string(),
            base_commit: "abc123".to_string(),
            applied_at_ms: 123,
            apply_result_hash: Some("hash-1".to_string()),
        };

        let serialized = serde_json::to_string(&evidence).unwrap();
        let deserialized: CheckpointAppliedEvidence = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, evidence);
    }

    #[test]
    fn test_checkpoint_lineage_state_round_trips() {
        let state = CheckpointLineageState {
            repo_workdir: "/repo".to_string(),
            current_epoch: 7,
            updated_at_ms: 999,
        };

        let serialized = serde_json::to_string(&state).unwrap();
        let deserialized: CheckpointLineageState = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, state);
    }
}
