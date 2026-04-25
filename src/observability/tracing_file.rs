use crate::config;
use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use std::sync::Once;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::time::FormatTime;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

struct LocalTimer;

impl FormatTime for LocalTimer {
    fn format_time(&self, w: &mut Writer<'_>) -> std::fmt::Result {
        write!(w, "{}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"))
    }
}

static COMMAND_TRACING_INIT: Once = Once::new();

struct FileMakeWriter {
    file: File,
}

impl<'a> MakeWriter<'a> for FileMakeWriter {
    type Writer = File;

    fn make_writer(&'a self) -> Self::Writer {
        self.file
            .try_clone()
            .expect("command tracing log file clone should succeed")
    }
}

fn command_tracing_internal_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("GIT_AI_DAEMON_HOME")
        && !home.trim().is_empty()
    {
        return Some(PathBuf::from(home).join(".git-ai").join("internal"));
    }

    config::internal_dir_path()
}

fn command_tracing_dir() -> Option<PathBuf> {
    command_tracing_internal_dir().map(|dir| dir.join("tracing").join("commands"))
}

fn command_tracing_log_path(command_kind: &str) -> Option<PathBuf> {
    // yyyy-mm-dd
    let date = chrono::Local::now().format("%Y-%m-%d").to_string();
    command_tracing_dir().map(|dir| dir.join(format!("{}-{}.log", command_kind, date)))
}

pub fn init_command_tracing(command_kind: &'static str) {
    COMMAND_TRACING_INIT.call_once(|| {
        let Some(log_path) = command_tracing_log_path(command_kind) else {
            return;
        };

        if let Some(parent) = log_path.parent()
            && fs::create_dir_all(parent).is_err()
        {
            return;
        }

        let Ok(file) = OpenOptions::new().create(true).append(true).open(&log_path) else {
            return;
        };

        let env_filter = if std::env::var("GIT_AI_DEBUG").as_deref() == Ok("1") {
            EnvFilter::new("debug")
        } else {
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"))
        };

        let _ = tracing_subscriber::registry()
            .with(env_filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_timer(LocalTimer)
                    .with_target(false)
                    .with_thread_ids(false)
                    .with_ansi(false)
                    .with_writer(FileMakeWriter { file }),
            )
            .try_init();
    });
}
