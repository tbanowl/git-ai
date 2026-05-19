use crate::commands::git_handlers::CommandHooksContext;
use crate::commands::hooks::commit_hooks::get_commit_default_author;
use crate::commands::hooks::plumbing_rewrite_hooks::apply_wrapper_plumbing_rewrite_if_possible;
use crate::git::cli_parser::ParsedGitInvocation;
use crate::git::repository::Repository;
use git2::{Oid, Repository as Git2Repository};

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedUpdateRefCommand {
    ref_name: String,
    no_deref: bool,
}

pub fn pre_update_ref_hook(
    parsed_args: &ParsedGitInvocation,
    repository: &mut Repository,
    _context: &mut CommandHooksContext,
) {
    clear_pre_update_ref_state(repository);

    let Some(command) = parse_simple_update_ref(parsed_args) else {
        return;
    };
    if command.no_deref && command.ref_name == "HEAD" {
        tracing::debug!(
            "Skipping update-ref tracking for --no-deref HEAD: symbolic-HEAD-only updates are intentionally left unhandled"
        );
        return;
    }
    if !should_track_ref_update(&command.ref_name) {
        return;
    }

    let current_head_ref = repository
        .head()
        .ok()
        .and_then(|head| head.name().map(|name| name.to_string()));
    let affects_checked_out_branch = command.ref_name == "HEAD"
        || current_head_ref.as_deref() == Some(command.ref_name.as_str());

    repository.pre_update_ref_refname = Some(command.ref_name.clone());
    repository.pre_update_ref_old_target = resolve_ref_target(repository, &command.ref_name);
    repository.pre_update_ref_affects_checked_out_branch = Some(affects_checked_out_branch);
}

pub fn post_update_ref_hook(
    parsed_args: &ParsedGitInvocation,
    repository: &mut Repository,
    exit_status: std::process::ExitStatus,
    _context: &mut CommandHooksContext,
) {
    if !exit_status.success() {
        clear_pre_update_ref_state(repository);
        return;
    }

    let Some(ref_name) = repository.pre_update_ref_refname.clone() else {
        clear_pre_update_ref_state(repository);
        return;
    };
    let old_target = repository.pre_update_ref_old_target.clone();
    let affects_checked_out_branch = repository
        .pre_update_ref_affects_checked_out_branch
        .unwrap_or(false);
    clear_pre_update_ref_state(repository);

    let Some(old_target) = old_target else {
        return;
    };

    let Some(new_target) = resolve_ref_target(repository, &ref_name) else {
        return;
    };

    if old_target == new_target {
        return;
    }

    if is_ancestor(repository, &old_target, &new_target) {
        if affects_checked_out_branch {
            let _ = repository
                .storage
                .rename_working_log(&old_target, &new_target);
        }
        return;
    }

    if is_ancestor(repository, &new_target, &old_target) {
        tracing::debug!(
            "Skipping wrapper update-ref rewind handling for {}: {} -> {}",
            ref_name,
            old_target,
            new_target
        );
        return;
    }

    let commit_author = get_commit_default_author(repository, &parsed_args.command_args);
    if !apply_wrapper_plumbing_rewrite_if_possible(
        repository,
        &old_target,
        &new_target,
        &commit_author,
        true,
    ) {
        tracing::debug!(
            "Skipping wrapper update-ref rewrite handling for {}: could not derive safe mappings",
            ref_name
        );
    }
}

fn clear_pre_update_ref_state(repository: &mut Repository) {
    repository.pre_update_ref_refname = None;
    repository.pre_update_ref_old_target = None;
    repository.pre_update_ref_affects_checked_out_branch = None;
}

fn parse_simple_update_ref(parsed_args: &ParsedGitInvocation) -> Option<ParsedUpdateRefCommand> {
    if parsed_args.command.as_deref() != Some("update-ref") {
        return None;
    }

    let args = &parsed_args.command_args;
    let mut positionals = Vec::new();
    let mut no_deref = false;
    let mut i = 0usize;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--stdin" | "--batch-updates" | "-d" | "--delete" => return None,
            "-m" | "--message" => {
                if i + 1 >= args.len() {
                    return None;
                }
                i += 2;
                continue;
            }
            "--create-reflog" => {
                i += 1;
                continue;
            }
            "--no-deref" => {
                no_deref = true;
                i += 1;
                continue;
            }
            _ if arg.starts_with("--message=") => {
                i += 1;
                continue;
            }
            _ if arg.starts_with('-') => return None,
            _ => {
                positionals.push(arg.clone());
                i += 1;
            }
        }
    }

    match positionals.as_slice() {
        [ref_name, _new_oid] => Some(ParsedUpdateRefCommand {
            ref_name: ref_name.clone(),
            no_deref,
        }),
        [ref_name, _new_oid, _old_oid] => Some(ParsedUpdateRefCommand {
            ref_name: ref_name.clone(),
            no_deref,
        }),
        _ => None,
    }
}

fn should_track_ref_update(ref_name: &str) -> bool {
    ref_name == "HEAD" || ref_name.starts_with("refs/heads/")
}

fn resolve_ref_target(repository: &Repository, ref_name: &str) -> Option<String> {
    repository
        .revparse_single(ref_name)
        .and_then(|obj| obj.peel_to_commit())
        .map(|commit| commit.id())
        .ok()
}

fn is_ancestor(repository: &Repository, ancestor: &str, descendant: &str) -> bool {
    // Migrated from: git merge-base --is-ancestor <ancestor> <descendant>
    // Backend: git2
    let Ok(g2repo) = Git2Repository::open(repository.path()) else {
        return false;
    };
    let Ok(ancestor_oid) = Oid::from_str(ancestor) else {
        return false;
    };
    let Ok(descendant_oid) = Oid::from_str(descendant) else {
        return false;
    };
    if ancestor_oid == descendant_oid {
        return true;
    }
    g2repo
        .graph_descendant_of(descendant_oid, ancestor_oid)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::parse_simple_update_ref;
    use crate::git::cli_parser::parse_git_cli_args;

    #[test]
    fn parses_simple_update_ref() {
        let parsed = parse_git_cli_args(&[
            "update-ref".to_string(),
            "refs/heads/topic".to_string(),
            "abc123".to_string(),
        ]);
        let command = parse_simple_update_ref(&parsed).expect("should parse");
        assert_eq!(command.ref_name, "refs/heads/topic");
        assert!(!command.no_deref);
    }

    #[test]
    fn rejects_update_ref_stdin_mode() {
        let parsed = parse_git_cli_args(&["update-ref".to_string(), "--stdin".to_string()]);
        assert!(parse_simple_update_ref(&parsed).is_none());
    }

    #[test]
    fn rejects_update_ref_no_deref_head_mode() {
        let parsed = parse_git_cli_args(&[
            "update-ref".to_string(),
            "--no-deref".to_string(),
            "HEAD".to_string(),
            "abc123".to_string(),
        ]);
        let command = parse_simple_update_ref(&parsed)
            .expect("parser should still recognize simple no-deref form");
        assert_eq!(command.ref_name, "HEAD");
        assert!(
            command.no_deref,
            "parser should preserve --no-deref so the hook can safely decline handling symbolic HEAD updates"
        );
    }
}
