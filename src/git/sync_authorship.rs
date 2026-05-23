use crate::api::types::AuthorshipNotesListItem;
use crate::git::refs::{
    AI_AUTHORSHIP_PUSH_REFSPEC, CommitAuthorship, copy_ref, fallback_merge_notes_ours,
    get_commits_with_notes_from_list, merge_notes_from_ref, note_blob_oids_for_commits,
    notes_add_batch, ref_exists, show_authorship_note, tracking_ref_for_remote,
};
use crate::{
    api::{ApiClient, ApiContext},
    config::Config,
    error::GitAiError,
    git::{cli_parser::ParsedGitInvocation, repository::exec_git},
    repo_url::normalize_repo_url,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::repository::Repository;

const REST_NOTES_SYNC_STATE_SCHEMA_VERSION: u32 = 1;
const REST_NOTES_SYNC_LIST_LIMIT: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RestNotesSyncState {
    pub schema_version: u32,
    pub repo_url: String,
    pub last_change_seq: i64,
    pub updated_at: i64,
}

pub fn sha256_note_content(content: &str) -> String {
    sha256_bytes(content.as_bytes())
}

fn sha256_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("{:x}", hasher.finalize())
}

pub fn normalize_content_hash(hash: &str) -> Result<String, GitAiError> {
    let hex = match hash.find(':') {
        Some(colon_pos) => {
            let prefix = &hash[..colon_pos];
            let body = &hash[colon_pos + 1..];
            if prefix.eq_ignore_ascii_case("sha256") {
                body
            } else {
                return Err(GitAiError::Generic(format!(
                    "Unsupported hash algorithm in '{}', expected sha256",
                    hash
                )));
            }
        }
        None => hash,
    };

    let lowered = hex.to_ascii_lowercase();
    if lowered.len() == 64 && lowered.chars().all(|c: char| c.is_ascii_hexdigit()) {
        Ok(lowered)
    } else {
        Err(GitAiError::Generic(format!(
            "Invalid content hash (expected 64-char hex, got '{}')",
            hash
        )))
    }
}

pub fn rest_notes_repo_key(repo_url: &str) -> Result<String, GitAiError> {
    let normalized = normalize_repo_url(repo_url).map_err(|e| {
        GitAiError::Generic(format!("Cannot normalize repo URL '{}': {}", repo_url, e))
    })?;
    Ok(sha256_note_content(&normalized))
}

pub fn rest_notes_sync_state_path(git_dir: &Path, repo_url: &str) -> Result<PathBuf, GitAiError> {
    let key = rest_notes_repo_key(repo_url)?;
    Ok(git_dir
        .join("ai")
        .join("rest_notes_sync_state")
        .join(format!("{}.json", key.replace(['/', ':'], "_"))))
}

pub fn current_time_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

pub fn default_rest_notes_sync_state(repo_url: &str) -> RestNotesSyncState {
    RestNotesSyncState {
        schema_version: REST_NOTES_SYNC_STATE_SCHEMA_VERSION,
        repo_url: repo_url.to_string(),
        last_change_seq: 0,
        updated_at: 0,
    }
}

pub fn read_rest_notes_sync_state(
    git_dir: &Path,
    repo_url: &str,
) -> Result<RestNotesSyncState, GitAiError> {
    let normalized = normalize_repo_url(repo_url).map_err(|e| {
        GitAiError::Generic(format!("Cannot normalize repo URL '{}': {}", repo_url, e))
    })?;
    let path = rest_notes_sync_state_path(git_dir, repo_url)?;

    if !path.exists() {
        return Ok(default_rest_notes_sync_state(&normalized));
    }

    let data = std::fs::read_to_string(&path)?;
    let state: RestNotesSyncState = serde_json::from_str(&data)?;

    if state.repo_url != normalized || state.schema_version != REST_NOTES_SYNC_STATE_SCHEMA_VERSION
    {
        return Ok(default_rest_notes_sync_state(&normalized));
    }

    Ok(state)
}

pub fn write_rest_notes_sync_state(
    git_dir: &Path,
    repo_url: &str,
    new_last_change_seq: i64,
) -> Result<(), GitAiError> {
    let normalized = normalize_repo_url(repo_url).map_err(|e| {
        GitAiError::Generic(format!("Cannot normalize repo URL '{}': {}", repo_url, e))
    })?;
    let path = rest_notes_sync_state_path(git_dir, repo_url)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let _lock = StateFileLock::acquire(&path)?;

    let effective_watermark = compute_watermark(&path, &normalized, new_last_change_seq)?;

    let tmp_path = path.with_extension(format!(
        "json.tmp.{}.{}",
        std::process::id(),
        current_time_millis()
    ));

    let state = RestNotesSyncState {
        schema_version: REST_NOTES_SYNC_STATE_SCHEMA_VERSION,
        repo_url: normalized,
        last_change_seq: effective_watermark,
        updated_at: current_time_millis(),
    };

    {
        let mut file = std::fs::File::create(&tmp_path)?;
        let json = serde_json::to_string_pretty(&state)?;
        file.write_all(json.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
    }

    std::fs::rename(&tmp_path, &path)?;

    Ok(())
}

fn compute_watermark(
    path: &Path,
    normalized_url: &str,
    requested_seq: i64,
) -> Result<i64, GitAiError> {
    if !path.exists() {
        return Ok(requested_seq);
    }
    let data = std::fs::read_to_string(path)?;
    let state: RestNotesSyncState = serde_json::from_str(&data)?;
    if state.repo_url == normalized_url
        && state.schema_version == REST_NOTES_SYNC_STATE_SCHEMA_VERSION
    {
        Ok(state.last_change_seq.max(requested_seq))
    } else {
        Ok(requested_seq)
    }
}

struct StateFileLock {
    lock_path: PathBuf,
}

impl StateFileLock {
    fn acquire(state_path: &Path) -> Result<Self, GitAiError> {
        let lock_path = state_path.with_extension("json.lock");
        let parent = lock_path.parent().ok_or_else(|| {
            GitAiError::Generic("State file path has no parent directory".to_string())
        })?;
        std::fs::create_dir_all(parent)?;

        let max_attempts = 50;
        let sleep_duration = Duration::from_millis(20);
        let mut last_err = None;

        for _ in 0..max_attempts {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(_file) => return Ok(Self { lock_path }),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_err = Some(e);
                    std::thread::sleep(sleep_duration);
                }
                Err(e) => {
                    return Err(GitAiError::IoError(e));
                }
            }
        }

        Err(GitAiError::Generic(format!(
            "Timed out acquiring lock on {} (held by another process): {}",
            lock_path.display(),
            last_err
                .map(|e| e.to_string())
                .unwrap_or_else(|| "unknown".to_string())
        )))
    }
}

impl Drop for StateFileLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

#[cfg(windows)]
fn disabled_hooks_config() -> &'static str {
    "core.hooksPath=NUL"
}

#[cfg(not(windows))]
fn disabled_hooks_config() -> &'static str {
    "core.hooksPath=/dev/null"
}

/// Result of checking for authorship notes on a remote
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotesExistence {
    /// Notes were found and fetched from the remote
    Found,
    /// Confirmed that no notes exist on the remote
    NotFound,
}

pub fn fetch_remote_from_args(
    repository: &Repository,
    parsed_args: &ParsedGitInvocation,
) -> Result<String, GitAiError> {
    let remotes = repository.remotes().ok();
    let remote_names: Vec<String> = remotes
        .as_ref()
        .map(|r| {
            (0..r.len())
                .filter_map(|i| r.get(i).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // 2) Fetch authorship refs from the appropriate remote
    // Try to detect remote (named remote, URL, or local path) from args first
    let positional_remote = extract_remote_from_fetch_args(&parsed_args.command_args);
    let specified_remote = positional_remote.or_else(|| {
        parsed_args
            .command_args
            .iter()
            .find(|a| remote_names.iter().any(|r| r == *a))
            .cloned()
    });

    let remote = specified_remote
        .or_else(|| repository.upstream_remote().ok().flatten())
        .or_else(|| repository.get_default_remote().ok().flatten());

    remote.map(|r| r.to_string()).ok_or_else(|| {
        GitAiError::Generic(
            "Could not determine a remote for fetch/push operation. \
                 No remote was specified in args, no upstream is configured, \
                 and no default remote was found."
                .to_string(),
        )
    })
}

/// Try to fetch authorship notes from all remotes for source commits that are missing
/// local notes.  This is a best-effort operation used before cherry-pick attribution
/// rewriting to ensure notes from remote repos are available locally.
///
/// Uses the safe fetch pattern (tracking ref + merge with `-s ours`) so local notes
/// are never overwritten.  Silently ignores any fetch errors.
pub fn fetch_missing_notes_for_commits(repository: &Repository, source_commits: &[String]) {
    use std::collections::HashSet;

    // Fetch the full set of locally-noted commits in one subprocess call.
    // `git notes --ref=refs/notes/ai list` outputs "<note-sha> <commit-sha>" per line.
    let mut args = repository.global_args_for_exec();
    args.extend(
        ["notes", "--ref=refs/notes/ai", "list"]
            .iter()
            .map(|s| s.to_string()),
    );
    let noted_commits: HashSet<String> = exec_git(&args)
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|line| line.split_whitespace().nth(1).map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let missing: Vec<&String> = source_commits
        .iter()
        .filter(|sha| !noted_commits.contains(sha.as_str()))
        .collect();

    if missing.is_empty() {
        return;
    }

    tracing::debug!(
        "Source commits missing notes: {:?}, trying to fetch from remotes",
        missing
    );

    if let Ok(remotes) = repository.remotes_with_urls() {
        for (remote_name, _) in remotes {
            tracing::debug!("Attempting safe notes fetch from remote {}", remote_name);
            match fetch_authorship_notes(repository, &remote_name) {
                Ok(_) => tracing::debug!("✓ Fetched and merged notes from remote {}", remote_name),
                Err(e) => tracing::debug!(
                    "Notes fetch from remote {} failed (best-effort): {}",
                    remote_name,
                    e
                ),
            }
        }
    }
}

// for use with post-fetch and post-pull and post-clone hooks
// Returns Ok(NotesExistence::Found) if notes were found and fetched,
// Ok(NotesExistence::NotFound) if confirmed no notes exist on remote,
// Err(...) for actual errors (network, permissions, etc.)
pub fn fetch_authorship_notes(
    repository: &Repository,
    remote_name: &str,
) -> Result<NotesExistence, GitAiError> {
    if Config::fresh().notes_store() == "rest" {
        // Fetch notes via REST API instead of git fetch, if configured
        let api = ApiClient::new(ApiContext::new(None));
        let normalized_repo_url = normalized_rest_repo_url(repository, remote_name)?;
        return rest_fetch_authorship_notes(repository, &api, &normalized_repo_url);
    }
    // Generate tracking ref for this remote
    let tracking_ref = tracking_ref_for_remote(remote_name);

    tracing::debug!(
        "fetching authorship notes for remote '{}' to tracking ref '{}'",
        remote_name,
        tracking_ref
    );

    // Fetch notes to tracking ref with explicit refspec.
    // If the remote does not have refs/notes/ai yet, treat that as NotFound.
    let fetch_refspec = format!("+refs/notes/ai:{}", tracking_ref);

    // Build the internal authorship fetch with explicit flags and disabled hooks.
    // IMPORTANT: use repository.global_args_for_exec() to ensure -C flag is present for bare repos.
    let fetch_authorship = build_authorship_fetch_args(
        repository.global_args_for_exec(),
        remote_name,
        &fetch_refspec,
    );

    tracing::debug!("fetch command: {:?}", fetch_authorship);

    match exec_git(&fetch_authorship) {
        Ok(output) => {
            tracing::debug!(
                "fetch stdout: '{}'",
                String::from_utf8_lossy(&output.stdout)
            );
            tracing::debug!(
                "fetch stderr: '{}'",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Err(e) => {
            if is_missing_remote_notes_ref_error(&e) {
                tracing::debug!(
                    "no authorship notes found on remote '{}', nothing to sync",
                    remote_name
                );
                return Ok(NotesExistence::NotFound);
            }
            tracing::debug!("authorship fetch failed: {}", e);
            return Err(e);
        }
    }

    // After successful fetch, merge the tracking ref into refs/notes/ai
    let local_notes_ref = "refs/notes/ai";

    if crate::git::refs::ref_exists(repository, &tracking_ref) {
        if crate::git::refs::ref_exists(repository, local_notes_ref) {
            // Both exist - merge them
            tracing::debug!(
                "merging authorship notes from {} into {}",
                tracking_ref,
                local_notes_ref
            );
            if let Err(e) = merge_notes_from_ref(repository, &tracking_ref) {
                tracing::debug!("notes merge failed: {}", e);
                // Fallback: manually merge notes when git notes merge crashes
                if let Err(e2) = fallback_merge_notes_ours(repository, &tracking_ref) {
                    tracing::debug!("fallback merge also failed: {}", e2);
                }
            }
        } else {
            // Only tracking ref exists - copy it to local
            tracing::debug!(
                "initializing {} from tracking ref {}",
                local_notes_ref,
                tracking_ref
            );
            if let Err(e) = copy_ref(repository, &tracking_ref, local_notes_ref) {
                tracing::debug!("notes copy failed: {}", e);
                // Don't fail on copy errors, just log and continue
            }
        }
    } else {
        tracing::debug!("tracking ref {} was not created after fetch", tracking_ref);
    }

    Ok(NotesExistence::Found)
}

fn is_missing_remote_notes_ref_error(error: &GitAiError) -> bool {
    let GitAiError::GitCliError { stderr, .. } = error else {
        return false;
    };

    let stderr_lower = stderr.to_ascii_lowercase();
    stderr_lower.contains("refs/notes/ai")
        && (stderr_lower.contains("couldn't find remote ref")
            || stderr_lower.contains("could not find remote ref")
            || stderr_lower.contains("remote ref does not exist")
            || stderr_lower.contains("not our ref"))
}
/// Maximum number of fetch-merge-push attempts before giving up.
/// On busy monorepos, concurrent pushers can cause non-fast-forward rejections
/// even after a successful merge, so we retry the full cycle.
const PUSH_NOTES_MAX_ATTEMPTS: usize = 3;

// for use with post-push hook
pub fn push_authorship_notes(repository: &Repository, remote_name: &str) -> Result<(), GitAiError> {
    if Config::fresh().notes_store() == "rest" {
        let api = ApiClient::new(ApiContext::new(None));
        let normalized_repo_url = normalized_rest_repo_url(repository, remote_name)?;
        return rest_push_notes(repository, &api, &normalized_repo_url);
    }
    let mut last_error = None;

    for attempt in 0..PUSH_NOTES_MAX_ATTEMPTS {
        if attempt > 0 {
            tracing::debug!(
                "retrying notes push (attempt {}/{})",
                attempt + 1,
                PUSH_NOTES_MAX_ATTEMPTS
            );
        }

        fetch_and_merge_tracking_notes(repository, remote_name);

        // Push notes without force (requires fast-forward)
        let push_args = build_authorship_push_args(repository.global_args_for_exec(), remote_name);

        tracing::debug!("pushing authorship refs (no force): {:?}", &push_args);

        match exec_git(&push_args) {
            Ok(_) => return Ok(()),
            Err(e) => {
                tracing::debug!("authorship push failed: {}", e);
                if is_non_fast_forward_error(&e) && attempt + 1 < PUSH_NOTES_MAX_ATTEMPTS {
                    // Another pusher updated remote notes between our merge and push.
                    // Retry the full fetch-merge-push cycle.
                    last_error = Some(e);
                    continue;
                }
                return Err(e);
            }
        }
    }

    Err(last_error
        .unwrap_or_else(|| GitAiError::Generic("notes push exhausted retries".to_string())))
}

/// Fetch remote notes into a tracking ref and merge into local refs/notes/ai.
fn fetch_and_merge_tracking_notes(repository: &Repository, remote_name: &str) {
    if Config::fresh().notes_store() == "rest" {
        let api = ApiClient::new(ApiContext::new(None));
        let normalized_repo_url = match normalized_rest_repo_url(repository, remote_name) {
            Ok(repo_url) => repo_url,
            Err(e) => {
                tracing::debug!("pre-push REST notes remote resolution failed: {}", e);
                return;
            }
        };

        if let Err(e) = rest_fetch_authorship_notes(repository, &api, &normalized_repo_url) {
            tracing::debug!("pre-push REST notes fetch failed: {}", e);
        }
        return;
    }

    let tracking_ref = tracking_ref_for_remote(remote_name);
    let fetch_refspec = format!("+refs/notes/ai:{}", tracking_ref);

    let fetch_args = build_authorship_fetch_args(
        repository.global_args_for_exec(),
        remote_name,
        &fetch_refspec,
    );

    tracing::debug!("pre-push authorship fetch: {:?}", &fetch_args);

    // Fetch is best-effort; if it fails (e.g., no remote notes yet), continue
    if exec_git(&fetch_args).is_err() {
        return;
    }

    let local_notes_ref = "refs/notes/ai";

    if !ref_exists(repository, &tracking_ref) {
        return;
    }

    if !ref_exists(repository, local_notes_ref) {
        // Only tracking ref exists - copy it to local
        tracing::debug!(
            "pre-push: initializing {} from {}",
            local_notes_ref,
            tracking_ref
        );
        if let Err(e) = copy_ref(repository, &tracking_ref, local_notes_ref) {
            tracing::debug!("pre-push notes copy failed: {}", e);
        }
        return;
    }

    // Both exist - merge them
    tracing::debug!(
        "pre-push: merging {} into {}",
        tracking_ref,
        local_notes_ref
    );
    if let Err(e) = merge_notes_from_ref(repository, &tracking_ref) {
        tracing::debug!("pre-push notes merge failed: {}", e);
        // Fallback: manually merge notes when git notes merge crashes
        // (e.g., due to corrupted/mixed-fanout notes trees, or git bugs
        // with fanout-level mismatches on older git versions like macOS)
        if let Err(e2) = fallback_merge_notes_ours(repository, &tracking_ref) {
            tracing::debug!("pre-push fallback merge also failed: {}", e2);
        }
    }
}

fn is_non_fast_forward_error(error: &GitAiError) -> bool {
    let GitAiError::GitCliError { stderr, .. } = error else {
        return false;
    };
    stderr.contains("non-fast-forward")
}

fn extract_remote_from_fetch_args(args: &[String]) -> Option<String> {
    let mut after_double_dash = false;
    let mut skip_next_option_value = false;

    for arg in args {
        if skip_next_option_value {
            skip_next_option_value = false;
            continue;
        }

        if !after_double_dash {
            if arg == "--" {
                after_double_dash = true;
                continue;
            }
            if arg == "-C" {
                skip_next_option_value = true;
                continue;
            }
            if arg.starts_with('-') {
                // Option; skip
                continue;
            }
        }

        // Candidate positional arg; determine if it's a repository URL/path
        let s = arg.as_str();

        // 1) URL forms (https://, ssh://, file://, git://, etc.)
        if s.contains("://") || s.starts_with("file://") {
            return Some(arg.clone());
        }

        // 2) SCP-like syntax: user@host:path
        if s.contains('@') && s.contains(':') && !s.contains("://") {
            return Some(arg.clone());
        }

        // 3) Local path forms
        if s.starts_with('/') || s.starts_with("./") || s.starts_with("../") || s.starts_with("~/")
        {
            return Some(arg.clone());
        }

        // Heuristic: bare repo directories often end with .git
        if s.ends_with(".git") {
            return Some(arg.clone());
        }

        // 4) As a last resort, if the path exists on disk, treat as local path
        if std::path::Path::new(s).exists() {
            return Some(arg.clone());
        }

        // Otherwise, do not treat this positional token as a repository; likely a refspec
        break;
    }

    None
}

fn with_disabled_hooks(mut args: Vec<String>) -> Vec<String> {
    args.push("-c".to_string());
    args.push(disabled_hooks_config().to_string());
    args
}

fn build_authorship_fetch_args(
    global_args: Vec<String>,
    remote_name: &str,
    fetch_refspec: &str,
) -> Vec<String> {
    let mut args = with_disabled_hooks(global_args);
    args.push("fetch".to_string());
    args.push("--no-tags".to_string());
    args.push("--recurse-submodules=no".to_string());
    args.push("--no-write-fetch-head".to_string());
    args.push("--no-write-commit-graph".to_string());
    args.push("--no-auto-maintenance".to_string());
    args.push(remote_name.to_string());
    args.push(fetch_refspec.to_string());
    args
}

fn build_authorship_push_args(global_args: Vec<String>, remote_name: &str) -> Vec<String> {
    let mut args = with_disabled_hooks(global_args);
    args.push("push".to_string());
    args.push("--quiet".to_string());
    args.push("--no-recurse-submodules".to_string());
    args.push("--no-verify".to_string());
    args.push("--no-signed".to_string());
    args.push(remote_name.to_string());
    args.push(AI_AUTHORSHIP_PUSH_REFSPEC.to_string());
    args
}

fn resolve_remote_name_or_url(
    repository: &Repository,
    remote_name: &str,
) -> Result<String, GitAiError> {
    let candidate = remote_name.trim();

    let looks_like_url_or_path = candidate.contains("://")
        || candidate.starts_with("file://")
        || (candidate.contains('@') && candidate.contains(':') && !candidate.contains("://"))
        || candidate.starts_with('/')
        || candidate.starts_with("./")
        || candidate.starts_with("../")
        || candidate.starts_with("~/")
        || candidate.ends_with(".git")
        || std::path::Path::new(candidate).exists();

    if looks_like_url_or_path {
        return Ok(candidate.to_string());
    }

    let remotes = repository.remotes_with_urls()?;
    remotes
        .into_iter()
        .find(|(name, _)| name == candidate)
        .map(|(_, url)| url)
        .ok_or_else(|| {
            GitAiError::Generic(format!(
                "Could not resolve remote '{}' to a URL for REST notes sync",
                candidate
            ))
        })
}

fn normalized_rest_repo_url(
    repository: &Repository,
    remote_name: &str,
) -> Result<String, GitAiError> {
    let remote_url = resolve_remote_name_or_url(repository, remote_name)?;
    normalize_repo_url(&remote_url).map_err(|e| {
        GitAiError::Generic(format!(
            "Invalid remote URL for REST notes sync '{}': {}",
            remote_url, e
        ))
    })
}

fn list_local_authorship_notes_with_blob_oid(
    repository: &Repository,
) -> Result<Vec<(String, String)>, GitAiError> {
    let mut args = repository.global_args_for_exec();
    args.push("notes".to_string());
    args.push("--ref=ai".to_string());
    args.push("list".to_string());

    let output = match exec_git(&args) {
        Ok(output) => output,
        Err(GitAiError::GitCliError { code: Some(1), .. }) => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let stdout = String::from_utf8(output.stdout)?;
    let mut mappings = Vec::new();
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let mut parts = line.split_whitespace();
        let Some(note_blob_oid) = parts.next() else {
            continue;
        };
        let Some(commit_sha) = parts.next() else {
            continue;
        };
        mappings.push((commit_sha.to_string(), note_blob_oid.to_string()));
    }

    Ok(mappings)
}

// fn derive_since_commit_time(repository: &Repository) -> Option<i64> {
//     let mut args = repository.global_args_for_exec();
//     args.push("rev-list".to_string());
//     args.push("--timestamp".to_string());
//     args.push("-1".to_string());
//     args.push("@{u}".to_string());

//     let output = exec_git(&args).ok()?;
//     let stdout = String::from_utf8(output.stdout).ok()?;
//     let line = stdout.lines().next()?;
//     let commit_time: i64 = line.split_whitespace().next()?.parse().ok()?;
//     Some(commit_time - 86400)
// }

fn rest_fetch_authorship_notes(
    repository: &Repository,
    api: &ApiClient,
    repo_url: &str,
) -> Result<NotesExistence, GitAiError> {
    let state = read_rest_notes_sync_state(repository.common_dir(), repo_url)?;
    let mut since_change_seq = state.last_change_seq;
    let mut final_change_seq;
    let mut saw_remote_note = false;

    loop {
        let list_response =
            api.authorship_notes_list(&crate::api::types::AuthorshipNotesListRequest {
                repo_url: repo_url.to_string(),
                since_commit_time: None,
                since_change_seq: Some(since_change_seq),
                limit: Some(REST_NOTES_SYNC_LIST_LIMIT),
            })?;

        let Some(items) = list_response.data.items else {
            return rest_fetch_authorship_notes_legacy(repository, api, repo_url);
        };

        if items.is_empty() && !list_response.data.commit_shas.is_empty() {
            return rest_fetch_authorship_notes_legacy(repository, api, repo_url);
        }

        if !items.is_empty() {
            saw_remote_note = true;
        }

        let page_next_change_seq = validate_next_change_seq(
            &items,
            list_response.data.next_change_seq,
            since_change_seq,
            list_response.data.has_more == Some(true),
        )?;
        let mut expected_hashes = HashMap::<String, String>::new();
        let mut needs_batch = Vec::<String>::new();

        for item in &items {
            let expected_hash = normalize_content_hash(&item.content_hash)?;
            expected_hashes.insert(item.commit_sha.clone(), expected_hash.clone());

            let local_hash_matches = show_authorship_note(repository, &item.commit_sha)
                .map(|content| sha256_note_content(&content) == expected_hash)
                .unwrap_or(false);
            if !local_hash_matches {
                needs_batch.push(item.commit_sha.clone());
            }
        }

        if !needs_batch.is_empty() {
            let batch_response: crate::api::AuthorshipBatchResponse = api
                .authorship_notes_batch_get(&crate::api::types::AuthorshipNotesBatchRequest {
                    repo_url: repo_url.to_string(),
                    commit_shas: needs_batch.clone(),
                })?;

            let requested_set: HashSet<String> = needs_batch.iter().cloned().collect();
            let missing_from_batch: Vec<String> = batch_response
                .data
                .missing
                .iter()
                .filter(|commit_sha| requested_set.contains(*commit_sha))
                .cloned()
                .collect();
            if !missing_from_batch.is_empty() {
                return Err(GitAiError::Generic(format!(
                    "REST notes batch missing commits from current page: {}",
                    missing_from_batch.join(", ")
                )));
            }

            let mut returned_notes = HashMap::<String, String>::new();
            for note in batch_response.data.notes {
                if !requested_set.contains(&note.commit_sha) {
                    continue;
                }
                let expected_hash = expected_hashes.get(&note.commit_sha).ok_or_else(|| {
                    GitAiError::Generic(format!(
                        "REST notes batch returned unexpected commit {}",
                        note.commit_sha
                    ))
                })?;
                if let Some(returned_hash) = note.content_hash.as_deref() {
                    let returned_hash = normalize_content_hash(returned_hash)?;
                    if &returned_hash != expected_hash {
                        return Err(GitAiError::Generic(format!(
                            "REST notes batch content hash mismatch for {}: expected {}, got {}",
                            note.commit_sha, expected_hash, returned_hash
                        )));
                    }
                }
                let actual_hash = sha256_note_content(&note.content);
                if &actual_hash != expected_hash {
                    return Err(GitAiError::Generic(format!(
                        "REST notes batch content hash mismatch for {}: expected {}, got {}",
                        note.commit_sha, expected_hash, actual_hash
                    )));
                }
                returned_notes.insert(note.commit_sha, note.content);
            }

            let mut entries = Vec::<(String, String)>::new();
            for commit_sha in &needs_batch {
                let content = returned_notes.remove(commit_sha).ok_or_else(|| {
                    GitAiError::Generic(format!(
                        "REST notes batch omitted commit from current page: {}",
                        commit_sha
                    ))
                })?;
                entries.push((commit_sha.clone(), content));
            }

            notes_add_batch(repository, &entries)?;
        }

        final_change_seq = page_next_change_seq;
        if list_response.data.has_more != Some(true) {
            break;
        }
        since_change_seq = page_next_change_seq;
    }

    write_rest_notes_sync_state(repository.common_dir(), repo_url, final_change_seq)?;

    if saw_remote_note {
        Ok(NotesExistence::Found)
    } else {
        Ok(NotesExistence::NotFound)
    }
}

fn validate_next_change_seq(
    items: &[AuthorshipNotesListItem],
    next_change_seq: Option<i64>,
    since_change_seq: i64,
    has_more: bool,
) -> Result<i64, GitAiError> {
    let max_item_change_seq = items.iter().map(|item| item.change_seq).max().unwrap_or(0);
    let next_change_seq = next_change_seq.unwrap_or(max_item_change_seq);
    if next_change_seq < max_item_change_seq {
        return Err(GitAiError::Generic(format!(
            "Invalid next_change_seq {} below max item change_seq {}",
            next_change_seq, max_item_change_seq
        )));
    }
    if has_more && next_change_seq <= since_change_seq {
        return Err(GitAiError::Generic(format!(
            "Invalid REST notes pagination: has_more=true but next_change_seq {} does not advance beyond since_change_seq {}",
            next_change_seq, since_change_seq
        )));
    }
    Ok(next_change_seq)
}

fn fetch_remote_note_hashes(
    api: &ApiClient,
    repo_url: &str,
) -> Result<HashMap<String, Option<String>>, GitAiError> {
    let mut since_change_seq = 0;
    let mut remote_hashes = HashMap::<String, Option<String>>::new();

    loop {
        let list_response =
            api.authorship_notes_list(&crate::api::types::AuthorshipNotesListRequest {
                repo_url: repo_url.to_string(),
                since_commit_time: None,
                since_change_seq: Some(since_change_seq),
                limit: Some(REST_NOTES_SYNC_LIST_LIMIT),
            })?;

        let Some(items) = list_response.data.items else {
            return Ok(list_response
                .data
                .commit_shas
                .into_iter()
                .map(|commit_sha| (commit_sha, None))
                .collect());
        };

        let page_next_change_seq = validate_next_change_seq(
            &items,
            list_response.data.next_change_seq,
            since_change_seq,
            list_response.data.has_more == Some(true),
        )?;

        for item in items {
            remote_hashes.insert(
                item.commit_sha,
                Some(normalize_content_hash(&item.content_hash)?),
            );
        }

        if list_response.data.has_more != Some(true) {
            break;
        }
        since_change_seq = page_next_change_seq;
    }

    Ok(remote_hashes)
}

fn rest_fetch_authorship_notes_legacy(
    repository: &Repository,
    api: &ApiClient,
    repo_url: &str,
) -> Result<NotesExistence, GitAiError> {
    // let since_commit_time = derive_since_commit_time(repository);
    let list_response =
        api.authorship_notes_list(&crate::api::types::AuthorshipNotesListRequest {
            repo_url: repo_url.to_string(),
            since_commit_time: None,
            since_change_seq: None,
            limit: None,
        })?;

    if list_response.data.commit_shas.is_empty() {
        return Ok(NotesExistence::NotFound);
    }

    let remote_commit_shas = list_response.data.commit_shas;
    let local_note_blob_oids = note_blob_oids_for_commits(repository, &remote_commit_shas)?;

    // Find commits that are missing or have different content
    let missing_or_changed: Vec<String> = remote_commit_shas
        .iter()
        .filter(|commit_sha| {
            // Commit is missing if local doesn't have this commit at all
            !local_note_blob_oids.contains_key(*commit_sha)
        })
        .cloned()
        .collect();

    if missing_or_changed.is_empty() {
        return Ok(NotesExistence::Found);
    }

    let batch_response: crate::api::AuthorshipBatchResponse =
        api.authorship_notes_batch_get(&crate::api::types::AuthorshipNotesBatchRequest {
            repo_url: repo_url.to_string(),
            commit_shas: missing_or_changed.clone(),
        })?;

    let missing_set: HashSet<String> = missing_or_changed.into_iter().collect();
    let entries: Vec<(String, String)> = batch_response
        .data
        .notes
        .into_iter()
        .filter(|note| missing_set.contains(&note.commit_sha))
        .map(|note| (note.commit_sha, note.content))
        .collect();

    if !entries.is_empty() {
        notes_add_batch(repository, &entries)?;
    }

    Ok(NotesExistence::Found)
}

fn rest_push_notes(
    repository: &Repository,
    api: &ApiClient,
    repo_url: &str,
) -> Result<(), GitAiError> {
    let local_notes = list_local_authorship_notes_with_blob_oid(repository)?;
    if local_notes.is_empty() {
        return Ok(());
    }

    let remote_note_hashes = fetch_remote_note_hashes(api, repo_url)?;

    let mut local_notes_to_push = Vec::<(String, String)>::new();
    for (sha, note_blob_oid) in local_notes {
        let should_push = match remote_note_hashes.get(&sha) {
            None => true,
            Some(None) => false,
            Some(Some(remote_hash)) => show_authorship_note(repository, &sha)
                .map(|content| sha256_note_content(&content) != *remote_hash)
                .unwrap_or(false),
        };
        if should_push {
            local_notes_to_push.push((sha, note_blob_oid));
        }
    }

    if local_notes_to_push.is_empty() {
        return Ok(());
    }

    let commits_to_push: Vec<String> = local_notes_to_push
        .iter()
        .map(|(sha, _)| sha.clone())
        .collect();

    let commit_authorships = get_commits_with_notes_from_list(repository, &commits_to_push)?;
    let mut commit_author_map: HashMap<String, (String, i64)> = HashMap::new();
    for authorship in commit_authorships {
        match authorship {
            CommitAuthorship::NoLog {
                sha,
                git_author,
                commit_time,
            }
            | CommitAuthorship::Log {
                sha,
                git_author,
                commit_time,
                ..
            } => {
                commit_author_map.insert(sha, (git_author, commit_time));
            }
        }
    }

    // Get current branch name using git command
    let branch = get_current_branch(repository).unwrap_or_else(|_| "main".to_string());

    let mut notes = Vec::new();
    for (commit_sha, note_blob_oid) in local_notes_to_push {
        let Some(content) = show_authorship_note(repository, &commit_sha) else {
            continue;
        };

        let (git_author, commit_time) = commit_author_map
            .get(&commit_sha)
            .cloned()
            .unwrap_or_else(|| ("Unknown <unknown@example.com>".to_string(), 0));

        let (author_name, author_email) = parse_author_identity(&git_author);

        notes.push(crate::api::types::AuthorshipNotesPushItem {
            branch: branch.clone(),
            commit_sha,
            note_blob_oid,
            author_name,
            author_email,
            content,
            commit_time,
        });
    }

    if notes.is_empty() {
        return Ok(());
    }

    api.authorship_notes_push(&crate::api::types::AuthorshipNotesPushRequest {
        repo_url: repo_url.to_string(),
        notes,
    })?;

    Ok(())
}

/// Parse git author identity from "Name <email>" format
fn parse_author_identity(author_str: &str) -> (String, String) {
    // Format is usually "Name <email>" or just "Name"
    let email_start = author_str.find('<');
    let email_end = author_str.find('>');

    match (email_start, email_end) {
        (Some(start), Some(end)) => {
            let name = author_str[..start].trim().to_string();
            let email = author_str[start + 1..end].trim().to_string();
            (name, email)
        }
        _ => (author_str.to_string(), String::new()),
    }
}

/// Get current branch name from repository using git rev-parse
fn get_current_branch(repository: &Repository) -> Result<String, GitAiError> {
    let g2repo = git2::Repository::open(repository.path())
        .map_err(|e| GitAiError::Generic(e.to_string()))?;
    let head = g2repo
        .head()
        .map_err(|e| GitAiError::Generic(e.to_string()))?;
    let branch = head.shorthand().unwrap_or("HEAD").trim().to_string();

    if branch.is_empty() || !head.is_branch() || branch == "HEAD" {
        Err(GitAiError::Generic("Not on a branch".to_string()))
    } else {
        Ok(branch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::git::test_utils::TmpRepo;
    use std::process::Command;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    #[test]
    fn authorship_fetch_args_always_disable_hooks() {
        let disabled_hooks = disabled_hooks_config();
        let args = build_authorship_fetch_args(
            vec!["-C".to_string(), "/tmp/repo".to_string()],
            "origin",
            "+refs/notes/ai:refs/notes/ai-remote/origin",
        );

        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "-c" && pair[1] == disabled_hooks)
        );
        assert!(args.contains(&"fetch".to_string()));
    }

    #[test]
    fn authorship_push_args_always_disable_hooks() {
        let disabled_hooks = disabled_hooks_config();
        let args =
            build_authorship_push_args(vec!["-C".to_string(), "/tmp/repo".to_string()], "origin");

        assert!(
            args.windows(2)
                .any(|pair| pair[0] == "-c" && pair[1] == disabled_hooks)
        );
        assert!(args.contains(&"push".to_string()));
    }

    #[test]
    fn missing_remote_notes_ref_error_is_detected() {
        let err = GitAiError::GitCliError {
            code: Some(128),
            stderr: "fatal: couldn't find remote ref refs/notes/ai".to_string(),
            args: vec!["fetch".to_string(), "origin".to_string()],
        };
        assert!(is_missing_remote_notes_ref_error(&err));
    }

    #[test]
    fn missing_remote_notes_ref_error_ignores_unrelated_git_errors() {
        let err = GitAiError::GitCliError {
            code: Some(128),
            stderr: "fatal: Authentication failed for 'https://github.com/org/repo.git/'"
                .to_string(),
            args: vec!["fetch".to_string(), "origin".to_string()],
        };
        assert!(!is_missing_remote_notes_ref_error(&err));
    }

    #[test]
    fn extract_remote_from_fetch_args_skips_c_flag_values() {
        let args = strings(&["-C", "/tmp/repo", "origin", "main"]);
        assert_eq!(extract_remote_from_fetch_args(&args), None);
    }

    #[test]
    fn extract_remote_from_fetch_args_detects_url_after_options() {
        let args = strings(&["--no-tags", "https://github.com/example/repo.git", "main"]);
        assert_eq!(
            extract_remote_from_fetch_args(&args).as_deref(),
            Some("https://github.com/example/repo.git")
        );
    }

    #[test]
    fn get_current_branch_returns_checked_out_branch_name() {
        let tmp_repo = TmpRepo::new().expect("create tmp repo");

        tmp_repo
            .write_file("tracked.txt", "content\n", true)
            .expect("write file");
        tmp_repo
            .commit_with_message("Initial commit")
            .expect("commit");

        let branch = get_current_branch(tmp_repo.gitai_repo()).expect("branch name");
        assert_eq!(branch, tmp_repo.current_branch().expect("current branch"));
    }

    #[test]
    fn get_current_branch_errors_for_detached_head() {
        let tmp_repo = TmpRepo::new().expect("create tmp repo");

        tmp_repo
            .write_file("tracked.txt", "content\n", true)
            .expect("write file");
        tmp_repo
            .commit_with_message("Initial commit")
            .expect("commit");

        let head_sha = tmp_repo.get_head_commit_sha().expect("head sha");
        let output = Command::new(Config::get().git_cmd())
            .current_dir(tmp_repo.path())
            .args(["checkout", &head_sha])
            .output()
            .expect("detach HEAD");
        assert!(
            output.status.success(),
            "git checkout should detach HEAD: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let err = get_current_branch(tmp_repo.gitai_repo())
            .expect_err("detached HEAD should produce an error")
            .to_string();
        assert!(err.contains("Not on a branch"), "unexpected error: {err}");
    }

    // ---- Incremental sync helper tests ----

    #[test]
    fn sha256_note_content_known_value() {
        assert_eq!(
            sha256_note_content("hello\n"),
            "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"
        );
    }

    #[test]
    fn sha256_note_content_empty() {
        assert_eq!(
            sha256_note_content(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn normalize_content_hash_plain_lowercase() {
        let hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert_eq!(normalize_content_hash(hex).unwrap(), hex);
    }

    #[test]
    fn normalize_content_hash_prefixed_uppercase() {
        let hex = "SHA256:0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF";
        let result = normalize_content_hash(hex).unwrap();
        assert_eq!(
            result,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        );
    }

    #[test]
    fn normalize_content_hash_prefixed_lowercase() {
        let hex = "sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let result = normalize_content_hash(hex).unwrap();
        assert_eq!(
            result,
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
    }

    #[test]
    fn normalize_content_hash_prefixed_mixed_case() {
        let hex = "ShA256:ABCDEF0123456789abcdef0123456789ABCDEF0123456789abcdef0123456789";
        let result = normalize_content_hash(hex).unwrap();
        assert_eq!(
            result,
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
        );
    }

    #[test]
    fn normalize_content_hash_rejects_wrong_algorithm() {
        assert!(
            normalize_content_hash(
                "md5:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
            )
            .is_err()
        );
    }

    #[test]
    fn normalize_content_hash_rejects_invalid() {
        assert!(normalize_content_hash("not-a-hash").is_err());
        assert!(
            normalize_content_hash(
                "ZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ"
            )
            .is_err()
        );
        assert!(normalize_content_hash("0123456789abcdef").is_err()); // too short
        assert!(normalize_content_hash("sha256").is_err()); // bare prefix, no colon body
        assert!(normalize_content_hash("SHA256").is_err());
        assert!(normalize_content_hash("sha256:").is_err()); // empty body
        assert!(normalize_content_hash("sha256:abc").is_err()); // body too short
    }

    #[test]
    fn rest_notes_sync_state_path_under_git_dir() {
        let git_dir = Path::new("/tmp/repo/.git");
        let url = "https://github.com/example/repo";
        let path = rest_notes_sync_state_path(git_dir, url).unwrap();
        assert!(path.starts_with("/tmp/repo/.git/ai/rest_notes_sync_state/"));
        assert!(path.extension().unwrap() == "json");
        let filename = path.file_name().unwrap().to_str().unwrap();
        assert!(!filename.contains('/'));
        assert!(!filename.contains(':'));

        let expected_key = rest_notes_repo_key(url).unwrap();
        let expected_path = git_dir
            .join("ai")
            .join("rest_notes_sync_state")
            .join(format!("{}.json", expected_key));
        assert_eq!(path, expected_path);
    }

    #[test]
    fn rest_notes_sync_state_path_equivalent_urls() {
        let git_dir = Path::new("/tmp/repo/.git");
        let path_git_suffix =
            rest_notes_sync_state_path(git_dir, "https://github.com/example/repo.git").unwrap();
        let path_trailing_slash =
            rest_notes_sync_state_path(git_dir, "https://github.com/example/repo/").unwrap();
        let path_ssh =
            rest_notes_sync_state_path(git_dir, "git@github.com:example/repo.git").unwrap();
        let path_bare =
            rest_notes_sync_state_path(git_dir, "https://github.com/example/repo").unwrap();

        assert_eq!(path_git_suffix, path_bare);
        assert_eq!(path_trailing_slash, path_bare);
        assert_eq!(path_ssh, path_bare);
    }

    #[test]
    fn rest_notes_repo_key_deterministic() {
        let url = "https://github.com/example/repo";
        let key1 = rest_notes_repo_key(url).unwrap();
        let key2 = rest_notes_repo_key(url).unwrap();
        assert_eq!(key1, key2);
        assert_eq!(key1.len(), 64);
        assert!(key1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn rest_notes_repo_key_different_urls() {
        let key1 = rest_notes_repo_key("https://github.com/example/repo1").unwrap();
        let key2 = rest_notes_repo_key("https://github.com/example/repo2").unwrap();
        assert_ne!(key1, key2);
    }

    #[test]
    fn current_time_millis_reasonable() {
        let ts = current_time_millis();
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        assert!(ts > 1_700_000_000_000); // after 2023
        assert!((ts - now_secs).unsigned_abs() < 5000); // within 5s
    }

    #[test]
    fn default_rest_notes_sync_state_fields() {
        let state = default_rest_notes_sync_state("https://github.com/example/repo");
        assert_eq!(state.schema_version, 1);
        assert_eq!(state.repo_url, "https://github.com/example/repo");
        assert_eq!(state.last_change_seq, 0);
        assert_eq!(state.updated_at, 0);
    }

    #[test]
    fn read_rest_notes_sync_state_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();
        let state = read_rest_notes_sync_state(git_dir, "https://example.com/repo").unwrap();
        assert_eq!(state.last_change_seq, 0);
        assert_eq!(state.repo_url, "https://example.com/repo");
    }

    #[test]
    fn read_rest_notes_sync_state_repo_url_mismatch_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();
        write_rest_notes_sync_state(git_dir, "https://example.com/repoA", 10).unwrap();
        let state = read_rest_notes_sync_state(git_dir, "https://example.com/repoB").unwrap();
        assert_eq!(state.last_change_seq, 0);
        assert_eq!(state.repo_url, "https://example.com/repoB");
    }

    #[test]
    fn read_rest_notes_sync_state_schema_mismatch_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();
        write_rest_notes_sync_state(git_dir, "https://example.com/repo", 10).unwrap();

        let path = rest_notes_sync_state_path(git_dir, "https://example.com/repo").unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        let tampered = raw.replace("\"schema_version\": 1", "\"schema_version\": 99");
        std::fs::write(&path, tampered).unwrap();

        let state = read_rest_notes_sync_state(git_dir, "https://example.com/repo").unwrap();
        assert_eq!(state.last_change_seq, 0);
    }

    #[test]
    fn write_rest_notes_sync_state_creates_dirs_and_file() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();
        write_rest_notes_sync_state(git_dir, "https://example.com/repo", 42).unwrap();

        let path = rest_notes_sync_state_path(git_dir, "https://example.com/repo").unwrap();
        assert!(path.exists());

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.starts_with('{'));
        assert!(content.ends_with("}\n"));
        let parsed: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(parsed["schema_version"], 1);
        assert_eq!(parsed["last_change_seq"], 42);
        assert!(parsed["updated_at"].is_number());
        assert!(parsed["updated_at"].as_i64().unwrap() > 0);
    }

    #[test]
    fn write_rest_notes_sync_state_monotonic_watermark() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();
        let url = "https://example.com/repo";

        write_rest_notes_sync_state(git_dir, url, 100).unwrap();
        write_rest_notes_sync_state(git_dir, url, 50).unwrap();

        let state = read_rest_notes_sync_state(git_dir, url).unwrap();
        assert_eq!(state.last_change_seq, 100); // did not move backwards
    }

    #[test]
    fn write_rest_notes_sync_state_advances_watermark() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();
        let url = "https://example.com/repo";

        write_rest_notes_sync_state(git_dir, url, 50).unwrap();
        write_rest_notes_sync_state(git_dir, url, 200).unwrap();

        let state = read_rest_notes_sync_state(git_dir, url).unwrap();
        assert_eq!(state.last_change_seq, 200);
    }

    #[test]
    fn write_and_read_equivalent_urls_share_state() {
        let dir = tempfile::tempdir().unwrap();
        let git_dir = dir.path();

        write_rest_notes_sync_state(git_dir, "https://github.com/example/repo.git", 42).unwrap();

        let state = read_rest_notes_sync_state(git_dir, "git@github.com:example/repo.git").unwrap();
        assert_eq!(state.last_change_seq, 42);
        assert_eq!(state.repo_url, "https://github.com/example/repo");

        write_rest_notes_sync_state(git_dir, "https://github.com/example/repo/", 100).unwrap();
        let state2 =
            read_rest_notes_sync_state(git_dir, "https://github.com/example/repo").unwrap();
        assert_eq!(state2.last_change_seq, 100);
    }

    #[test]
    fn validate_next_change_seq_accepts_next_at_or_above_max_item_change_seq() {
        let items = vec![
            crate::api::types::AuthorshipNotesListItem {
                commit_sha: "a".repeat(40),
                content_hash: "0".repeat(64),
                change_seq: 10,
                updated_at: 100,
            },
            crate::api::types::AuthorshipNotesListItem {
                commit_sha: "b".repeat(40),
                content_hash: "1".repeat(64),
                change_seq: 20,
                updated_at: 200,
            },
        ];

        assert_eq!(
            validate_next_change_seq(&items, Some(20), 0, false).unwrap(),
            20
        );
        assert_eq!(
            validate_next_change_seq(&items, Some(25), 0, true).unwrap(),
            25
        );
    }

    #[test]
    fn validate_next_change_seq_rejects_next_below_max_item_change_seq() {
        let items = vec![crate::api::types::AuthorshipNotesListItem {
            commit_sha: "a".repeat(40),
            content_hash: "0".repeat(64),
            change_seq: 20,
            updated_at: 100,
        }];

        let err = validate_next_change_seq(&items, Some(19), 0, false).unwrap_err();
        assert!(
            err.to_string().contains("next_change_seq"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn validate_next_change_seq_accepts_empty_final_page_without_progress() {
        assert_eq!(
            validate_next_change_seq(&[], Some(20), 20, false).unwrap(),
            20
        );
        assert_eq!(validate_next_change_seq(&[], None, 20, false).unwrap(), 0);
    }

    #[test]
    fn validate_next_change_seq_rejects_empty_has_more_page_without_progress() {
        let err = validate_next_change_seq(&[], Some(20), 20, true).unwrap_err();
        assert!(
            err.to_string().contains("does not advance"),
            "unexpected error: {}",
            err
        );

        let err = validate_next_change_seq(&[], None, 20, true).unwrap_err();
        assert!(
            err.to_string().contains("does not advance"),
            "unexpected error: {}",
            err
        );
    }

    #[test]
    fn concurrent_writers_cannot_move_watermark_backwards() {
        use std::sync::Arc;
        use std::thread;

        let dir = Arc::new(tempfile::tempdir().unwrap());
        let git_dir = dir.path().to_path_buf();
        let url = "https://github.com/example/repo";

        let max_seq = 200i64;
        let num_writers = 8;

        let handles: Vec<_> = (0..num_writers)
            .map(|i| {
                let git_dir = git_dir.clone();
                let url = url.to_string();
                thread::spawn(move || {
                    let seq = if i == 0 { max_seq } else { (i as i64) * 10 };
                    write_rest_notes_sync_state(&git_dir, &url, seq)
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap().unwrap();
        }

        let state = read_rest_notes_sync_state(&git_dir, url).unwrap();
        assert_eq!(state.last_change_seq, max_seq);
    }
}
