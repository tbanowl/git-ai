//! Handle flush-metrics-db command and auto-drain during flush-logs.
//!
//! Drains the metrics database queue by uploading batches to the API.

use crate::api::{ApiClient, ApiContext, upload_metrics_with_retry};
use crate::metrics::db::MetricsDatabase;
use crate::metrics::{MetricEvent, MetricsBatch};

/// Max events per batch upload
const MAX_BATCH_SIZE: usize = 250;

/// No-op in test mode.
#[cfg(any(test, feature = "test-support"))]
pub fn drain_metrics_db_backlog() {}

/// Drain the metrics-db backlog: read batches, upload, delete on success.
///
/// Skips silently if auth conditions are not met or database is unavailable.
/// On upload failure, stops and keeps remaining records for next attempt.
#[cfg(not(any(test, feature = "test-support")))]
pub fn drain_metrics_db_backlog() {
    let context = ApiContext::new(None);
    let api_base_url = context.base_url.clone();
    let client = ApiClient::new(context);

    let using_default_api = api_base_url == crate::config::DEFAULT_API_BASE_URL;
    if using_default_api && !client.is_logged_in() && !client.has_api_key() {
        return;
    }

    let db = match MetricsDatabase::global() {
        Ok(db) => db,
        Err(_) => return,
    };

    let mut total_uploaded = 0usize;
    let mut total_batches = 0usize;

    loop {
        let batch = {
            let db_lock = match db.lock() {
                Ok(lock) => lock,
                Err(_) => break,
            };
            match db_lock.get_batch(MAX_BATCH_SIZE) {
                Ok(batch) => batch,
                Err(_) => break,
            }
        };

        if batch.is_empty() {
            break;
        }

        let mut events = Vec::new();
        let mut record_ids = Vec::new();

        for record in &batch {
            if let Ok(event) = serde_json::from_str::<MetricEvent>(&record.event_json) {
                events.push(event);
                record_ids.push(record.id);
            } else if let Ok(mut db_lock) = db.lock() {
                let _ = db_lock.delete_records(&[record.id]);
            }
        }

        if events.is_empty() {
            continue;
        }

        let event_count = events.len();
        let metrics_batch = MetricsBatch::new(events);

        match upload_metrics_with_retry(&client, &metrics_batch, "flush_metrics_db") {
            Ok(()) => {
                total_uploaded += event_count;
                total_batches += 1;
                if let Ok(mut db_lock) = db.lock() {
                    let _ = db_lock.delete_records(&record_ids);
                }
            }
            Err(_) => break,
        }
    }

    if total_uploaded > 0 {
        eprintln!(
            "metrics-db drain: uploaded {} events in {} batch(es)",
            total_uploaded, total_batches
        );
    }
}

/// Handle the flush-metrics-db CLI command
pub fn handle_flush_metrics_db(_args: &[String]) {
    drain_metrics_db_backlog();
}
