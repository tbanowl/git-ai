use crate::git::refs::{
    AI_AUTHORSHIP_PUSH_REFSPEC, CommitAuthorship, copy_ref, get_commits_with_notes_from_list,
    merge_notes_from_ref, note_blob_oids_for_commits, notes_add_batch, ref_exists,
    show_authorship_note, tracking_ref_for_remote,
};
use crate::{
    api::{ApiClient, ApiContext},
    config::Config,
    error::GitAiError,
    git::{cli_parser::ParsedGitInvocation, repository::exec_git},
    repo_url::normalize_repo_url,
    utils::debug_log,
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

// for use with post-fetch and post-pull and post-clone hooks
// Returns Ok(NotesExistence::Found) if notes were found and fetched,
// Ok(NotesExistence::NotFound) if confirmed no notes exist on remote,
// Err(...) for actual errors (network, permissions, etc.)
pub fn fetch_authorship_notes(
    repository: &Repository,
    remote_name: &str,
) -> Result<NotesExistence, GitAiError> {
    if Config::get().notes_store() == "rest" {
        let api = ApiClient::new(ApiContext::new(None));
        let remote_url = resolve_remote_name_or_url(repository, remote_name)?;
        let normalized_repo_url = normalize_repo_url(&remote_url).map_err(|e| {
            GitAiError::Generic(format!(
                "Invalid remote URL for REST notes sync '{}': {}",
                remote_url, e
            ))
        })?;
        return rest_fetch_notes(repository, &api, &normalized_repo_url);
    }

    // Generate tracking ref for this remote
    let tracking_ref = tracking_ref_for_remote(remote_name);

    debug_log(&format!(
        "fetching authorship notes for remote '{}' to tracking ref '{}'",
        remote_name, tracking_ref
    ));

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

    debug_log(&format!("fetch command: {:?}", fetch_authorship));

    match exec_git(&fetch_authorship) {
        Ok(output) => {
            debug_log(&format!(
                "fetch stdout: '{}'",
                String::from_utf8_lossy(&output.stdout)
            ));
            debug_log(&format!(
                "fetch stderr: '{}'",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Err(e) => {
            if is_missing_remote_notes_ref_error(&e) {
                debug_log(&format!(
                    "no authorship notes found on remote '{}', nothing to sync",
                    remote_name
                ));
                return Ok(NotesExistence::NotFound);
            }
            debug_log(&format!("authorship fetch failed: {}", e));
            return Err(e);
        }
    }

    // After successful fetch, merge the tracking ref into refs/notes/ai
    let local_notes_ref = "refs/notes/ai";

    if crate::git::refs::ref_exists(repository, &tracking_ref) {
        if crate::git::refs::ref_exists(repository, local_notes_ref) {
            // Both exist - merge them
            debug_log(&format!(
                "merging authorship notes from {} into {}",
                tracking_ref, local_notes_ref
            ));
            if let Err(e) = merge_notes_from_ref(repository, &tracking_ref) {
                debug_log(&format!("notes merge failed: {}", e));
                // Don't fail on merge errors, just log and continue
            }
        } else {
            // Only tracking ref exists - copy it to local
            debug_log(&format!(
                "initializing {} from tracking ref {}",
                local_notes_ref, tracking_ref
            ));
            if let Err(e) = copy_ref(repository, &tracking_ref, local_notes_ref) {
                debug_log(&format!("notes copy failed: {}", e));
                // Don't fail on copy errors, just log and continue
            }
        }
    } else {
        debug_log(&format!(
            "tracking ref {} was not created after fetch",
            tracking_ref
        ));
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
// for use with post-push hook
pub fn push_authorship_notes(repository: &Repository, remote_name: &str) -> Result<(), GitAiError> {
    if Config::get().notes_store() == "rest" {
        let api = ApiClient::new(ApiContext::new(None));
        let remote_url = resolve_remote_name_or_url(repository, remote_name)?;
        let normalized_repo_url = normalize_repo_url(&remote_url).map_err(|e| {
            GitAiError::Generic(format!(
                "Invalid remote URL for REST notes sync '{}': {}",
                remote_url, e
            ))
        })?;
        return rest_push_notes(repository, &api, &normalized_repo_url);
    }

    // STEP 1: Fetch remote notes into tracking ref and merge before pushing
    // This ensures we don't lose notes from other branches/clones
    let tracking_ref = tracking_ref_for_remote(remote_name);
    let fetch_refspec = format!("+refs/notes/ai:{}", tracking_ref);

    let fetch_before_push = build_authorship_fetch_args(
        repository.global_args_for_exec(),
        remote_name,
        &fetch_refspec,
    );

    debug_log(&format!(
        "pre-push authorship fetch: {:?}",
        &fetch_before_push
    ));

    // Fetch is best-effort; if it fails (e.g., no remote notes yet), continue
    if exec_git(&fetch_before_push).is_ok() {
        // Merge fetched notes into local refs/notes/ai
        let local_notes_ref = "refs/notes/ai";

        if ref_exists(repository, &tracking_ref) {
            if ref_exists(repository, local_notes_ref) {
                // Both exist - merge them
                debug_log(&format!(
                    "pre-push: merging {} into {}",
                    tracking_ref, local_notes_ref
                ));
                if let Err(e) = merge_notes_from_ref(repository, &tracking_ref) {
                    debug_log(&format!("pre-push notes merge failed: {}", e));
                }
            } else {
                // Only tracking ref exists - copy it to local
                debug_log(&format!(
                    "pre-push: initializing {} from {}",
                    local_notes_ref, tracking_ref
                ));
                if let Err(e) = copy_ref(repository, &tracking_ref, local_notes_ref) {
                    debug_log(&format!("pre-push notes copy failed: {}", e));
                }
            }
        }
    }

    // STEP 2: Push notes without force (requires fast-forward)
    let push_authorship =
        build_authorship_push_args(repository.global_args_for_exec(), remote_name);

    debug_log(&format!(
        "pushing authorship refs (no force): {:?}",
        &push_authorship
    ));
    if let Err(e) = exec_git(&push_authorship) {
        // Best-effort; don't fail user operation due to authorship sync issues
        debug_log(&format!("authorship push skipped due to error: {}", e));
        return Err(e);
    }

    Ok(())
}

fn extract_remote_from_fetch_args(args: &[String]) -> Option<String> {
    let mut after_double_dash = false;

    for arg in args {
        if !after_double_dash {
            if arg == "--" {
                after_double_dash = true;
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

fn rest_fetch_notes(
    repository: &Repository,
    api: &ApiClient,
    repo_url: &str,
) -> Result<NotesExistence, GitAiError> {
    let list_response = api.notes_list(&crate::api::types::NotesListRequest {
        repo_url: repo_url.to_string(),
    })?;

    if list_response.data.notes.is_empty() {
        return Ok(NotesExistence::NotFound);
    }

    let remote_commit_shas: Vec<String> = list_response
        .data
        .notes
        .iter()
        .map(|note| note.commit_sha.clone())
        .collect();
    let local_note_blob_oids = note_blob_oids_for_commits(repository, &remote_commit_shas)?;

    let missing_or_changed: Vec<String> = list_response
        .data
        .notes
        .iter()
        .filter_map(|remote_note| {
            let local_blob_oid = local_note_blob_oids.get(&remote_note.commit_sha);
            if local_blob_oid == Some(&remote_note.note_blob_oid) {
                None
            } else {
                Some(remote_note.commit_sha.clone())
            }
        })
        .collect();

    if missing_or_changed.is_empty() {
        return Ok(NotesExistence::Found);
    }

    let batch_response = api.notes_batch_get(&crate::api::types::NotesBatchRequest {
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

    let local_blob_map: HashMap<String, String> = local_notes
        .iter()
        .map(|(commit_sha, blob_oid)| (commit_sha.clone(), blob_oid.clone()))
        .collect();

    let remote_notes = api
        .notes_list(&crate::api::types::NotesListRequest {
            repo_url: repo_url.to_string(),
        })?
        .data
        .notes;
    let remote_blob_map: HashMap<String, String> = remote_notes
        .into_iter()
        .map(|note| (note.commit_sha, note.note_blob_oid))
        .collect();

    let mut commits_to_push = Vec::new();
    for (commit_sha, local_blob_oid) in &local_blob_map {
        if remote_blob_map.get(commit_sha) != Some(local_blob_oid) {
            commits_to_push.push(commit_sha.clone());
        }
    }

    if commits_to_push.is_empty() {
        return Ok(());
    }

    let commit_authorships = get_commits_with_notes_from_list(repository, &commits_to_push)?;
    let mut commit_author_map = HashMap::new();
    for authorship in commit_authorships {
        match authorship {
            CommitAuthorship::NoLog { sha, git_author }
            | CommitAuthorship::Log {
                sha, git_author, ..
            } => {
                commit_author_map.insert(sha, git_author);
            }
        }
    }

    let mut notes = Vec::new();
    for commit_sha in commits_to_push {
        let Some(content) = show_authorship_note(repository, &commit_sha) else {
            continue;
        };
        let Some(note_blob_oid) = local_blob_map.get(&commit_sha) else {
            continue;
        };

        let git_author = commit_author_map
            .get(&commit_sha)
            .cloned()
            .unwrap_or_else(|| "Unknown".to_string());

        notes.push(crate::api::types::NotesPushItem {
            commit_sha,
            note_blob_oid: note_blob_oid.clone(),
            git_author,
            content,
        });
    }

    if notes.is_empty() {
        return Ok(());
    }

    api.notes_push(&crate::api::types::NotesPushRequest {
        repo_url: repo_url.to_string(),
        notes,
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
