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
use std::collections::{HashMap, HashSet};

use super::repository::Repository;

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
    if Config::get().notes_store() == "rest" {
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
    if Config::get().notes_store() == "rest" {
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
    if Config::get().notes_store() == "rest" {
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
    // let since_commit_time = derive_since_commit_time(repository);
    let list_response =
        api.authorship_notes_list(&crate::api::types::AuthorshipNotesListRequest {
            repo_url: repo_url.to_string(),
            since_commit_time: None,
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

    // Get remote commit_shas (list only returns commit_shas, not note_blob_oid)
    let remote_commit_shas = api
        .authorship_notes_list(&crate::api::types::AuthorshipNotesListRequest {
            repo_url: repo_url.to_string(),
            since_commit_time: None,
        })?
        .data
        .commit_shas;

    let remote_commit_set: HashSet<String> = remote_commit_shas.into_iter().collect();

    // Find commits that exist locally but not remotely
    let local_notes_to_push: Vec<(String, String)> = local_notes
        .into_iter()
        .filter(|(sha, _)| !remote_commit_set.contains(sha))
        .collect();

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
}
