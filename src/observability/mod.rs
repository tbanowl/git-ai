use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::metrics::MetricEvent;

pub mod flush;
pub mod tracing_file;
pub mod wrapper_performance_targets;
use crate::config;

/// Maximum events per metrics envelope
pub const MAX_METRICS_PER_ENVELOPE: usize = 250;

#[derive(Serialize, Deserialize, Clone)]
struct ErrorEnvelope {
    #[serde(rename = "type")]
    event_type: String,
    timestamp: String,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Clone)]
struct MessageEnvelope {
    #[serde(rename = "type")]
    event_type: String,
    timestamp: String,
    message: String,
    level: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize, Clone)]
struct PerformanceEnvelope {
    #[serde(rename = "type")]
    event_type: String,
    timestamp: String,
    operation: String,
    duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tags: Option<HashMap<String, String>>,
}

#[derive(Serialize, Deserialize, Clone)]
struct MetricsEnvelope {
    #[serde(rename = "type")]
    event_type: String,
    timestamp: String,
    version: u8,
    events: Vec<MetricEvent>,
}

#[derive(Clone)]
enum LogEnvelope {
    Error(ErrorEnvelope),
    Performance(PerformanceEnvelope),
    #[allow(dead_code)]
    Message(MessageEnvelope),
    #[allow(dead_code)]
    Metrics(MetricsEnvelope),
}

impl LogEnvelope {
    fn to_json(&self) -> Option<serde_json::Value> {
        match self {
            LogEnvelope::Error(e) => serde_json::to_value(e).ok(),
            LogEnvelope::Performance(p) => serde_json::to_value(p).ok(),
            LogEnvelope::Message(m) => serde_json::to_value(m).ok(),
            LogEnvelope::Metrics(m) => serde_json::to_value(m).ok(),
        }
    }
}

enum LogMode {
    Buffered(Vec<LogEnvelope>),
    Disk(PathBuf),
}

struct ObservabilityInner {
    mode: LogMode,
}

static OBSERVABILITY: OnceLock<Mutex<ObservabilityInner>> = OnceLock::new();
const ENV_FLUSH_LOGS_WORKER: &str = "GIT_AI_FLUSH_LOGS_WORKER";

fn get_observability() -> &'static Mutex<ObservabilityInner> {
    OBSERVABILITY.get_or_init(|| {
        // Initialize directly in Disk mode with global logs path
        // All logs go to ~/.git-ai/internal/logs/{PID}.log
        let mode = if let Some(home) = dirs::home_dir() {
            let logs_dir = home.join(".git-ai").join("internal").join("logs");
            if std::fs::create_dir_all(&logs_dir).is_ok() {
                LogMode::Disk(logs_dir.join(format!("{}.log", std::process::id())))
            } else {
                LogMode::Buffered(Vec::new())
            }
        } else {
            LogMode::Buffered(Vec::new())
        };
        Mutex::new(ObservabilityInner { mode })
    })
}

/// Append an envelope (buffer if no repo context, write to disk if context set).
///
/// In daemon mode (async_mode enabled), telemetry envelopes are sent over the
/// control socket instead of being written to disk. The daemon batches and
/// flushes them every 3 seconds.
fn append_envelope(envelope: LogEnvelope) {
    // In daemon mode, route through the control socket instead of disk.
    if crate::daemon::telemetry_handle::daemon_telemetry_available() {
        let telemetry_envelope = match &envelope {
            LogEnvelope::Error(e) => Some(crate::daemon::TelemetryEnvelope::Error {
                timestamp: e.timestamp.clone(),
                message: e.message.clone(),
                context: e.context.clone(),
            }),
            LogEnvelope::Performance(p) => Some(crate::daemon::TelemetryEnvelope::Performance {
                timestamp: p.timestamp.clone(),
                operation: p.operation.clone(),
                duration_ms: p.duration_ms,
                context: p.context.clone(),
                tags: p.tags.clone(),
            }),
            LogEnvelope::Message(m) => Some(crate::daemon::TelemetryEnvelope::Message {
                timestamp: m.timestamp.clone(),
                message: m.message.clone(),
                level: m.level.clone(),
                context: m.context.clone(),
            }),
            LogEnvelope::Metrics(m) => Some(crate::daemon::TelemetryEnvelope::Metrics {
                events: m.events.clone(),
            }),
        };
        if let Some(te) = telemetry_envelope {
            crate::daemon::telemetry_handle::submit_telemetry(vec![te]);
        }

        return;
    }

    let mut obs = get_observability().lock().unwrap();

    match &mut obs.mode {
        LogMode::Buffered(buffer) => {
            buffer.push(envelope);
            // Cap buffered envelopes to prevent unbounded memory growth
            // in long-running processes (e.g. daemon) when disk mode is unavailable.
            const MAX_BUFFERED_ENVELOPES: usize = 1024;
            if buffer.len() > MAX_BUFFERED_ENVELOPES {
                let excess = buffer.len() - MAX_BUFFERED_ENVELOPES;
                buffer.drain(0..excess);
            }
        }
        LogMode::Disk(log_path) => {
            let log_path = log_path.clone();
            drop(obs); // Release lock before file I/O

            if let Some(json) = envelope.to_json()
                && let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path)
            {
                let _ = writeln!(file, "{}", json);
            }
        }
    }
}

/// Submit telemetry envelopes via the best available daemon path:
/// 1. External daemon control socket (wrapper processes)
/// 2. In-process daemon telemetry worker (daemon process itself)
/// 3. No-op if daemon is not running (callers must handle non-daemon fallback)
fn submit_telemetry_envelope(envelopes: Vec<crate::daemon::TelemetryEnvelope>) {
    if crate::daemon::telemetry_handle::daemon_telemetry_available() {
        crate::daemon::telemetry_handle::submit_telemetry(envelopes);
    } else if crate::daemon::daemon_process_active() {
        crate::daemon::telemetry_worker::submit_daemon_internal_telemetry(envelopes);
    }
}

#[cfg(any(test, not(feature = "test-support")))]
fn build_metrics_log_envelopes(events: &[MetricEvent], timestamp: &str) -> Vec<MetricsEnvelope> {
    events
        .chunks(MAX_METRICS_PER_ENVELOPE)
        .map(|chunk| MetricsEnvelope {
            event_type: "metrics".to_string(),
            timestamp: timestamp.to_string(),
            version: crate::metrics::METRICS_API_VERSION,
            events: chunk.to_vec(),
        })
        .collect()
}

#[cfg(not(feature = "test-support"))]
fn route_metrics_events(events: Vec<MetricEvent>, async_mode: bool) {
    if events.is_empty() {
        return;
    }

    if async_mode {
        for chunk in events.chunks(MAX_METRICS_PER_ENVELOPE) {
            let envelope = crate::daemon::TelemetryEnvelope::Metrics {
                events: chunk.to_vec(),
            };
            submit_telemetry_envelope(vec![envelope]);
        }
        return;
    }

    let timestamp = chrono::Utc::now().to_rfc3339();
    for envelope in build_metrics_log_envelopes(&events, &timestamp) {
        append_envelope(LogEnvelope::Metrics(envelope));
    }
}

/// Log an error to Sentry (via daemon telemetry worker)
pub fn log_error(error: &dyn std::error::Error, context: Option<serde_json::Value>) {
    if config::Config::get().feature_flags().async_mode {
        let envelope = crate::daemon::TelemetryEnvelope::Error {
            timestamp: chrono::Utc::now().to_rfc3339(),
            message: error.to_string(),
            context: context.clone(),
        };
        submit_telemetry_envelope(vec![envelope]);
        return;
    }

    let envelope = ErrorEnvelope {
        event_type: "error".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        message: error.to_string(),
        context,
    };

    append_envelope(LogEnvelope::Error(envelope));
}

/// Log a performance metric to Sentry (via daemon telemetry worker)
pub fn log_performance(
    operation: &str,
    duration: Duration,
    context: Option<serde_json::Value>,
    tags: Option<HashMap<String, String>>,
) {
    if config::Config::get().feature_flags().async_mode {
        let envelope = crate::daemon::TelemetryEnvelope::Performance {
            timestamp: chrono::Utc::now().to_rfc3339(),
            operation: operation.to_string(),
            duration_ms: duration.as_millis(),
            context: context.clone(),
            tags: tags.clone(),
        };
        submit_telemetry_envelope(vec![envelope]);
        return;
    }

    let envelope = PerformanceEnvelope {
        event_type: "performance".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        operation: operation.to_string(),
        duration_ms: duration.as_millis(),
        context,
        tags,
    };
    append_envelope(LogEnvelope::Performance(envelope));
}

/// Log a message to Sentry (info, warning, etc.) (via daemon telemetry worker)
#[allow(dead_code)]
pub fn log_message(message: &str, level: &str, context: Option<serde_json::Value>) {
    if config::Config::get().feature_flags().async_mode {
        let envelope = crate::daemon::TelemetryEnvelope::Message {
            timestamp: chrono::Utc::now().to_rfc3339(),
            message: message.to_string(),
            level: level.to_string(),
            context,
        };
        submit_telemetry_envelope(vec![envelope]);
        return;
    }
    let envelope = MessageEnvelope {
        event_type: "message".to_string(),
        timestamp: chrono::Utc::now().to_rfc3339(),
        message: message.to_string(),
        level: level.to_string(),
        context,
    };
    append_envelope(LogEnvelope::Message(envelope));
}

/// Spawn a background process to flush logs to Sentry.
///
/// In daemon mode, this is a no-op because the daemon's telemetry worker
/// handles batched flushing on a 3-second interval.
pub fn spawn_background_flush() {
    // In daemon mode the telemetry worker handles flushing — skip the subprocess.
    if crate::daemon::telemetry_handle::daemon_telemetry_available() {
        return;
    }

    // Skip flush in test builds to prevent race conditions during test cleanup.
    // Tests spawn git-ai as a subprocess which calls this function. If the background
    // flush process is still starting when TestRepo::drop() runs, file handles may
    // remain open causing "Directory not empty" errors. Tests set GIT_AI_TEST_DB_PATH
    // to isolate their database, so we use that as the test detection mechanism.
    // This check is compiled out of release builds since tests only run in debug mode.
    #[cfg(debug_assertions)]
    if std::env::var("GIT_AI_TEST_DB_PATH").is_ok() || std::env::var("GITAI_TEST_DB_PATH").is_ok() {
        return;
    }

    if !should_spawn_background_flush() {
        return;
    }
    smol::block_on(async {
        let _ = crate::utils::spawn_internal_git_ai_subcommand(
            "flush-logs",
            &[],
            ENV_FLUSH_LOGS_WORKER,
            &[],
        );
    });
}

/// Debounce background flushes to avoid process/request storms when checkpoints
/// run in quick succession. In background agents, always flush immediately.
fn should_spawn_background_flush() -> bool {
    if crate::utils::is_in_background_agent() {
        return true;
    }

    const MIN_FLUSH_INTERVAL_SECS: u64 = 60;

    let Some(home) = dirs::home_dir() else {
        return true;
    };
    let internal_dir = home.join(".git-ai").join("internal");
    let _ = std::fs::create_dir_all(&internal_dir);

    let marker = internal_dir.join("last_flush_trigger_ts");
    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if let Ok(previous) = std::fs::read_to_string(&marker)
        && let Ok(previous_secs) = previous.trim().parse::<u64>()
        && now_secs.saturating_sub(previous_secs) < MIN_FLUSH_INTERVAL_SECS
    {
        return false;
    }

    let _ = std::fs::write(&marker, now_secs.to_string());
    true
}

/// Log a batch of metric events (via daemon telemetry worker).
///
/// Events are batched into envelopes of up to 250 events each.
pub fn log_metrics(
    #[cfg_attr(any(test, feature = "test-support"), allow(unused))] events: Vec<MetricEvent>,
) {
    #[cfg(any(test, feature = "test-support"))]
    return;

    #[cfg(not(any(test, feature = "test-support")))]
    {
        route_metrics_events(events, config::Config::get().feature_flags().async_mode);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;
    use std::time::Duration;

    fn sample_metric_event(sequence: u32) -> MetricEvent {
        let mut values = HashMap::new();
        values.insert("0".to_string(), json!(sequence));

        let mut attrs = HashMap::new();
        attrs.insert("0".to_string(), json!("1.3.1"));

        MetricEvent {
            timestamp: sequence,
            event_id: 1,
            values,
            attrs,
        }
    }

    // Test error logging
    #[test]
    fn test_log_error_no_panic() {
        use std::io;
        let error = io::Error::new(io::ErrorKind::NotFound, "test error");
        log_error(&error, None);
    }

    #[test]
    fn test_log_error_with_context() {
        use serde_json::json;
        use std::io;
        let error = io::Error::new(io::ErrorKind::PermissionDenied, "access denied");
        let context = json!({"file": "test.txt", "operation": "read"});
        log_error(&error, Some(context));
    }

    // Test performance logging
    #[test]
    fn test_log_performance_basic() {
        log_performance("test_operation", Duration::from_millis(100), None, None);
    }

    #[test]
    fn test_log_performance_with_context() {
        use serde_json::json;
        let context = json!({"files": 5, "lines": 100});
        log_performance("test_op", Duration::from_secs(1), Some(context), None);
    }

    #[test]
    fn test_log_performance_with_tags() {
        let mut tags = HashMap::new();
        tags.insert("command".to_string(), "commit".to_string());
        tags.insert("repo".to_string(), "test".to_string());
        log_performance("commit_op", Duration::from_millis(500), None, Some(tags));
    }

    // Test message logging
    #[test]
    fn test_log_message_basic() {
        log_message("test message", "info", None);
    }

    #[test]
    fn test_log_message_with_context() {
        use serde_json::json;
        let context = json!({"user": "test", "action": "login"});
        log_message("user logged in", "info", Some(context));
    }

    #[test]
    fn test_log_message_warning() {
        log_message("warning message", "warning", None);
    }

    // Test metrics logging
    #[test]
    fn test_log_metrics_empty() {
        log_metrics(vec![]);
    }

    #[test]
    fn test_build_metrics_log_envelopes_sets_metrics_metadata() {
        let envelopes =
            build_metrics_log_envelopes(&[sample_metric_event(1)], "2026-04-21T00:00:00Z");

        assert_eq!(envelopes.len(), 1);
        assert_eq!(envelopes[0].event_type, "metrics");
        assert_eq!(envelopes[0].timestamp, "2026-04-21T00:00:00Z");
        assert_eq!(envelopes[0].version, crate::metrics::METRICS_API_VERSION);
        assert_eq!(envelopes[0].events.len(), 1);
    }

    #[test]
    fn test_build_metrics_log_envelopes_chunks_large_batches() {
        let events: Vec<_> = (0..(MAX_METRICS_PER_ENVELOPE as u32 + 1))
            .map(sample_metric_event)
            .collect();

        let envelopes = build_metrics_log_envelopes(&events, "2026-04-21T00:00:00Z");

        assert_eq!(envelopes.len(), 2);
        assert_eq!(envelopes[0].events.len(), MAX_METRICS_PER_ENVELOPE);
        assert_eq!(envelopes[1].events.len(), 1);
    }

    // Test constants
    #[test]
    fn test_max_metrics_per_envelope() {
        assert_eq!(MAX_METRICS_PER_ENVELOPE, 250);
    }
}
