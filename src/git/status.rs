use crate::error::GitAiError;
use crate::git::repository::{InternalGitProfile, Repository, exec_git_with_profile};
use gix_index::entry::Stage;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::str;
use unicode_normalization::UnicodeNormalization;

/// Normalize a path string to NFC form so that decomposed (NFD) filenames
/// from macOS match precomposed (NFC) paths used internally.
fn nfc_path(path: String) -> String {
    if path.is_ascii() {
        return path;
    }
    path.nfc().collect()
}

/// Maximum number of pathspec arguments to pass on the command line.
/// Beyond this threshold, we run git without pathspecs and post-filter
/// in Rust to avoid OS `ARG_MAX` / E2BIG errors.
pub const MAX_PATHSPEC_ARGS: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusCode {
    Unmodified,
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    Unmerged,
    Untracked,
    Ignored,
    Unknown(char),
}

impl From<char> for StatusCode {
    fn from(value: char) -> Self {
        match value {
            '.' => StatusCode::Unmodified,
            'M' => StatusCode::Modified,
            'A' => StatusCode::Added,
            'D' => StatusCode::Deleted,
            'R' => StatusCode::Renamed,
            'C' => StatusCode::Copied,
            'U' => StatusCode::Unmerged,
            '?' => StatusCode::Untracked,
            '!' => StatusCode::Ignored,
            other => StatusCode::Unknown(other),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Ordinary,
    Rename,
    Copy,
    Unmerged,
    Untracked,
    Ignored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusEntry {
    pub path: String,
    pub staged: StatusCode,
    pub unstaged: StatusCode,
    pub kind: EntryKind,
    pub orig_path: Option<String>,
}

#[derive(Debug, Clone)]
struct StatusPathspecPlan {
    combined_pathspecs: HashSet<String>,
    should_full_scan: bool,
    needs_post_filter: bool,
}

#[derive(Debug, Clone)]
struct CollectedStatusRecord {
    path: String,
    staged: StatusCode,
    unstaged: StatusCode,
    kind: EntryKind,
    orig_path: Option<String>,
}

fn build_status_pathspec_plan(
    staged_filenames: HashSet<String>,
    pathspecs: Option<&HashSet<String>>,
) -> StatusPathspecPlan {
    let combined_pathspecs: HashSet<String> = if let Some(paths) = pathspecs {
        staged_filenames.union(paths).cloned().collect()
    } else {
        staged_filenames
    };

    let should_full_scan = pathspecs.is_none() && combined_pathspecs.is_empty();
    let has_non_ascii = combined_pathspecs.iter().any(|path| !path.is_ascii());
    let needs_post_filter =
        !should_full_scan && (combined_pathspecs.len() > MAX_PATHSPEC_ARGS || has_non_ascii);

    StatusPathspecPlan {
        combined_pathspecs,
        should_full_scan,
        needs_post_filter,
    }
}

fn post_filter_status_entries(entries: &mut Vec<StatusEntry>, combined_pathspecs: &HashSet<String>) {
    let nfc_pathspecs: HashSet<String> = combined_pathspecs
        .iter()
        .map(|path| nfc_path(path.clone()))
        .collect();

    entries.retain(|entry| {
        nfc_pathspecs.contains(&entry.path)
            || entry
                .orig_path
                .as_ref()
                .is_some_and(|orig_path| nfc_pathspecs.contains(orig_path))
    });
}

pub fn old_cli_status_entries_for_test(
    repository: &Repository,
    pathspecs: Option<&HashSet<String>>,
    skip_untracked: bool,
) -> Result<Vec<StatusEntry>, GitAiError> {
    let staged_filenames = repository.get_staged_filenames()?;
    let plan = build_status_pathspec_plan(staged_filenames, pathspecs);

    if plan.combined_pathspecs.is_empty() && !plan.should_full_scan {
        return Ok(Vec::new());
    }

    let mut args = repository.global_args_for_exec();
    args.push("--no-optional-locks".to_string());
    args.push("status".to_string());
    args.push("--porcelain=v2".to_string());
    args.push("-z".to_string());

    if skip_untracked {
        args.push("--untracked-files=no".to_string());
    }

    if !plan.should_full_scan && !plan.needs_post_filter && !plan.combined_pathspecs.is_empty() {
        args.push("--".to_string());
        for path in &plan.combined_pathspecs {
            args.push(path.clone());
        }
    }

    let output = exec_git_with_profile(&args, InternalGitProfile::General)?;

    if !output.status.success() {
        return Err(GitAiError::Generic(format!(
            "git status exited with status {}",
            output.status
        )));
    }

    let mut entries = parse_porcelain_v2(&output.stdout)?;

    if plan.needs_post_filter {
        post_filter_status_entries(&mut entries, &plan.combined_pathspecs);
    }

    Ok(entries)
}

fn collect_status_records(
    repository: &Repository,
    plan: &StatusPathspecPlan,
    skip_untracked: bool,
) -> Result<Vec<CollectedStatusRecord>, GitAiError> {
    let repo =
        git2::Repository::open(repository.path()).map_err(|e| GitAiError::Generic(e.to_string()))?;

    let mut options = git2::StatusOptions::new();
    options
        .include_untracked(!skip_untracked)
        .recurse_untracked_dirs(true)
        .include_ignored(true)
        .renames_head_to_index(true)
        .renames_index_to_workdir(true)
        .include_unmodified(false);

    let statuses = repo
        .statuses(Some(&mut options))
        .map_err(|e| GitAiError::Generic(e.to_string()))?;

    let mut entries = Vec::new();
    for entry in statuses.iter() {
        let Some(mut path) = entry.path().map(|path| nfc_path(path.to_string())) else {
            continue;
        };
        if path.trim().is_empty() {
            continue;
        }

        let status = entry.status();

        if status.is_conflicted() {
            continue;
        }

        if status.is_ignored() {
            entries.push(CollectedStatusRecord {
                path,
                staged: StatusCode::Unmodified,
                unstaged: StatusCode::Ignored,
                kind: EntryKind::Ignored,
                orig_path: None,
            });
            continue;
        }

        if status.is_wt_new() {
            entries.push(CollectedStatusRecord {
                path,
                staged: StatusCode::Unmodified,
                unstaged: StatusCode::Untracked,
                kind: EntryKind::Untracked,
                orig_path: None,
            });
            continue;
        }

        let mut staged = StatusCode::Unmodified;
        let mut unstaged = StatusCode::Unmodified;
        let mut kind = EntryKind::Ordinary;
        let mut orig_path = None;

        if status.intersects(
            git2::Status::INDEX_NEW
                | git2::Status::INDEX_MODIFIED
                | git2::Status::INDEX_DELETED
                | git2::Status::INDEX_RENAMED
                | git2::Status::INDEX_TYPECHANGE,
        ) {
            staged = if status.is_index_new() {
                StatusCode::Added
            } else if status.is_index_deleted() {
                StatusCode::Deleted
            } else if status.is_index_modified() || status.is_index_typechange() {
                StatusCode::Modified
            } else if status.is_index_renamed() {
                kind = EntryKind::Rename;
                StatusCode::Renamed
            } else {
                StatusCode::Unmodified
            };
        }

        if status.intersects(
            git2::Status::WT_MODIFIED
                | git2::Status::WT_DELETED
                | git2::Status::WT_RENAMED
                | git2::Status::WT_TYPECHANGE,
        ) {
            unstaged = if status.is_wt_deleted() {
                StatusCode::Deleted
            } else if status.is_wt_modified() || status.is_wt_typechange() {
                StatusCode::Modified
            } else if status.is_wt_renamed() {
                kind = EntryKind::Rename;
                StatusCode::Renamed
            } else {
                StatusCode::Unmodified
            };
        }

        if let Some(delta) = entry.head_to_index() {
            if delta.status() == git2::Delta::Renamed {
                kind = EntryKind::Rename;
                staged = StatusCode::Renamed;
                if let Some(new_path) = delta.new_file().path() {
                    path = nfc_path(new_path.to_string_lossy().replace('\\', "/"));
                }
                orig_path = delta
                    .old_file()
                    .path()
                    .map(|path| nfc_path(path.to_string_lossy().replace('\\', "/")));
            }
        }

        if let Some(delta) = entry.index_to_workdir() {
            if delta.status() == git2::Delta::Renamed {
                kind = EntryKind::Rename;
                unstaged = StatusCode::Renamed;
                if let Some(new_path) = delta.new_file().path() {
                    path = nfc_path(new_path.to_string_lossy().replace('\\', "/"));
                }
                orig_path = delta
                    .old_file()
                    .path()
                    .map(|path| nfc_path(path.to_string_lossy().replace('\\', "/")));
            }
        }

        entries.push(CollectedStatusRecord {
            path,
            staged,
            unstaged,
            kind,
            orig_path,
        });
    }

    if !plan.should_full_scan && !plan.needs_post_filter {
        entries.retain(|entry| {
            plan.combined_pathspecs.contains(&entry.path)
                || entry
                    .orig_path
                    .as_ref()
                    .is_some_and(|orig_path| plan.combined_pathspecs.contains(orig_path))
        });
    }

    Ok(entries)
}

fn assemble_status_entries(records: Vec<CollectedStatusRecord>) -> Vec<StatusEntry> {
    records
        .into_iter()
        .map(|record| StatusEntry {
            path: record.path,
            staged: record.staged,
            unstaged: record.unstaged,
            kind: record.kind,
            orig_path: record.orig_path,
        })
        .collect()
}

impl Repository {
    // Get status for tracked files that changed
    pub fn get_staged_filenames(&self) -> Result<HashSet<String>, GitAiError> {
        let object_hash =
            crate::git::repository::repository_object_hash_kind_for_path_no_git_exec(self.path())?;
        let index_path = self.path().join("index");
        let index = match gix_index::File::at(index_path, object_hash, true, Default::default()) {
            Ok(index) => index,
            Err(_) => return Ok(HashSet::new()),
        };

        let mut index_entries = HashMap::new();
        let mut conflict_paths = HashSet::new();
        for entry in index.entries() {
            let file_path = nfc_path(entry.path(&index).to_string());
            if file_path.trim().is_empty() {
                continue;
            }

            if entry.stage() == Stage::Unconflicted {
                index_entries.insert(file_path, (entry.id.to_string(), entry.mode.bits() as i32));
            } else {
                conflict_paths.insert(file_path);
            }
        }

        let repo =
            git2::Repository::open(self.path()).map_err(|e| GitAiError::Generic(e.to_string()))?;
        let head_entries = collect_head_tree_entries(&repo)?;

        let mut filenames = conflict_paths;
        for (path, (oid, mode)) in &index_entries {
            match head_entries.get(path) {
                Some((head_oid, head_mode)) if head_oid == oid && head_mode == mode => {}
                _ => {
                    filenames.insert(path.clone());
                }
            }
        }

        for path in head_entries.keys() {
            if !index_entries.contains_key(path) {
                filenames.insert(path.clone());
            }
        }

        Ok(filenames)
    }

    // Get status for tracked files that changed
    pub fn get_staged_and_unstaged_filenames(&self) -> Result<HashSet<String>, GitAiError> {
        let repo =
            git2::Repository::open(self.path()).map_err(|e| GitAiError::Generic(e.to_string()))?;
        let mut options = git2::StatusOptions::new();
        options
            .include_untracked(true)
            .recurse_untracked_dirs(true)
            .include_ignored(false)
            .renames_head_to_index(false)
            .renames_index_to_workdir(false);

        let statuses = repo
            .statuses(Some(&mut options))
            .map_err(|e| GitAiError::Generic(e.to_string()))?;

        let mut filenames = HashSet::new();
        for entry in statuses.iter() {
            if let Some(path) = entry.path() {
                let normalized = nfc_path(path.to_string());
                if !normalized.trim().is_empty() {
                    filenames.insert(normalized);
                }
            }
        }

        Ok(filenames)
    }

    pub fn status(
        &self,
        pathspecs: Option<&HashSet<String>>,
        skip_untracked: bool,
    ) -> Result<Vec<StatusEntry>, GitAiError> {
        let status_start = std::time::Instant::now();
        let staged_filenames = self.get_staged_filenames()?;
        let plan = build_status_pathspec_plan(staged_filenames, pathspecs);

        if plan.combined_pathspecs.is_empty() && !plan.should_full_scan {
            return Ok(Vec::new());
        }

        let records = collect_status_records(self, &plan, skip_untracked)?;
        let mut entries = assemble_status_entries(records);

        if plan.needs_post_filter {
            post_filter_status_entries(&mut entries, &plan.combined_pathspecs);
        }
        tracing::debug!(
            "Computed status entries in {:?} (pathspecs={:?}, skip_untracked={}, full_scan={}, post_filter={})",
            status_start.elapsed(),
            pathspecs,
            skip_untracked,
            plan.should_full_scan,
            plan.needs_post_filter
        );

        Ok(entries)
    }
}

fn collect_head_tree_entries(
    repo: &git2::Repository,
) -> Result<HashMap<String, (String, i32)>, GitAiError> {
    let mut entries = HashMap::new();
    let head = match repo.head() {
        Ok(head) => head,
        Err(err) if err.code() == git2::ErrorCode::UnbornBranch => return Ok(entries),
        Err(err) => return Err(GitAiError::Generic(err.to_string())),
    };
    let commit = head
        .peel_to_commit()
        .map_err(|e| GitAiError::Generic(e.to_string()))?;
    let tree = commit
        .tree()
        .map_err(|e| GitAiError::Generic(e.to_string()))?;
    collect_tree_entries(repo, &tree, Path::new(""), &mut entries)?;
    Ok(entries)
}

fn collect_tree_entries(
    repo: &git2::Repository,
    tree: &git2::Tree<'_>,
    prefix: &Path,
    entries: &mut HashMap<String, (String, i32)>,
) -> Result<(), GitAiError> {
    for entry in tree {
        let Ok(name) = str::from_utf8(entry.name_bytes()) else {
            continue;
        };
        let path = prefix.join(name);
        if entry.kind() == Some(git2::ObjectType::Tree) {
            let subtree = repo
                .find_tree(entry.id())
                .map_err(|e| GitAiError::Generic(e.to_string()))?;
            collect_tree_entries(repo, &subtree, &path, entries)?;
            continue;
        }

        let Some(path_str) = path.to_str() else {
            continue;
        };
        entries.insert(
            nfc_path(path_str.replace('\\', "/")),
            (entry.id().to_string(), entry.filemode()),
        );
    }
    Ok(())
}

fn parse_porcelain_v2(data: &[u8]) -> Result<Vec<StatusEntry>, GitAiError> {
    let mut entries = Vec::new();
    let mut parts = data
        .split(|byte| *byte == 0)
        .filter(|slice| !slice.is_empty())
        .peekable();

    while let Some(raw) = parts.next() {
        let record = str::from_utf8(raw)?;
        let mut chars = record.chars();
        let tag = chars
            .next()
            .ok_or_else(|| GitAiError::Generic("Unexpected empty porcelain v2 record".into()))?;

        match tag {
            '1' | 'u' => {
                let mut fields = record.splitn(9, ' ');
                let _ = fields.next(); // tag
                let xy = fields
                    .next()
                    .ok_or_else(|| GitAiError::Generic("Missing XY field".into()))?;
                if xy.len() != 2 {
                    return Err(GitAiError::Generic(format!(
                        "Unexpected XY field length: {}",
                        xy
                    )));
                }
                let staged = StatusCode::from(xy.chars().next().unwrap());
                let unstaged = StatusCode::from(xy.chars().nth(1).unwrap());

                // skip submodule/metadata fields to capture path
                for _ in 0..6 {
                    fields.next();
                }

                let path = nfc_path(
                    fields
                        .next()
                        .ok_or_else(|| GitAiError::Generic("Missing path field".into()))?
                        .to_string(),
                );

                entries.push(StatusEntry {
                    path,
                    staged,
                    unstaged,
                    kind: if matches!(staged, StatusCode::Unmerged)
                        || matches!(unstaged, StatusCode::Unmerged)
                    {
                        EntryKind::Unmerged
                    } else {
                        EntryKind::Ordinary
                    },
                    orig_path: None,
                });
            }
            '2' => {
                let mut fields = record.splitn(10, ' ');
                let _ = fields.next(); // tag
                let xy = fields
                    .next()
                    .ok_or_else(|| GitAiError::Generic("Missing XY field".into()))?;
                if xy.len() != 2 {
                    return Err(GitAiError::Generic(format!(
                        "Unexpected XY field length: {}",
                        xy
                    )));
                }
                let staged = StatusCode::from(xy.chars().next().unwrap());
                let unstaged = StatusCode::from(xy.chars().nth(1).unwrap());

                // skip submodule/metadata fields
                for _ in 0..7 {
                    fields.next();
                }

                let path = nfc_path(
                    fields
                        .next()
                        .ok_or_else(|| GitAiError::Generic("Missing path field".into()))?
                        .to_string(),
                );

                let orig_path_bytes = parts.next().ok_or_else(|| {
                    GitAiError::Generic("Missing original path for rename/copy".into())
                })?;
                let orig_path = nfc_path(str::from_utf8(orig_path_bytes)?.to_string());

                let kind = match staged {
                    StatusCode::Renamed => EntryKind::Rename,
                    StatusCode::Copied => EntryKind::Copy,
                    _ => EntryKind::Ordinary,
                };

                entries.push(StatusEntry {
                    path,
                    staged,
                    unstaged,
                    kind,
                    orig_path: Some(orig_path),
                });
            }
            '?' => {
                let path = nfc_path(record.strip_prefix("? ").unwrap_or(record).to_string());

                entries.push(StatusEntry {
                    path,
                    staged: StatusCode::Unmodified,
                    unstaged: StatusCode::Untracked,
                    kind: EntryKind::Untracked,
                    orig_path: None,
                });
            }
            '!' => {
                let path = nfc_path(record.strip_prefix("! ").unwrap_or(record).to_string());

                entries.push(StatusEntry {
                    path,
                    staged: StatusCode::Unmodified,
                    unstaged: StatusCode::Ignored,
                    kind: EntryKind::Ignored,
                    orig_path: None,
                });
            }
            other => {
                return Err(GitAiError::Generic(format!(
                    "Unsupported porcelain v2 record tag: {}",
                    other
                )));
            }
        }
    }

    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_debug_snapshot;
    use std::collections::HashSet;

    fn set(paths: &[&str]) -> HashSet<String> {
        paths.iter().map(|path| (*path).to_string()).collect()
    }

    #[test]
    fn pathspec_plan_unions_staged_paths_and_enables_post_filter_for_non_ascii() {
        let staged = set(&["staged.txt"]);
        let explicit = set(&["unicodé/文件.txt"]);

        let plan = build_status_pathspec_plan(staged, Some(&explicit));

        assert_eq!(
            plan.combined_pathspecs,
            set(&["staged.txt", "unicodé/文件.txt"])
        );
        assert!(!plan.should_full_scan);
        assert!(plan.needs_post_filter);
    }

    #[test]
    fn post_filter_status_entries_matches_path_and_orig_path_after_nfc_normalization() {
        let mut entries = vec![
            StatusEntry {
                path: "new.txt".to_string(),
                staged: StatusCode::Renamed,
                unstaged: StatusCode::Unmodified,
                kind: EntryKind::Rename,
                orig_path: Some("old.txt".to_string()),
            },
            StatusEntry {
                path: "space dir/hello world.txt".to_string(),
                staged: StatusCode::Unmodified,
                unstaged: StatusCode::Modified,
                kind: EntryKind::Ordinary,
                orig_path: None,
            },
        ];

        let filter = set(&["old.txt", "space dir/hello world.txt"]);
        post_filter_status_entries(&mut entries, &filter);

        assert_eq!(entries.len(), 2);
        assert!(entries.iter().any(|entry| entry.path == "new.txt"));
        assert!(
            entries
                .iter()
                .any(|entry| entry.path == "space dir/hello world.txt")
        );
    }

    #[test]
    fn parse_varied_porcelain_v2_records() {
        // Construct a blob of porcelain v2 entries covering tracked, renamed, copied,
        // unmerged, untracked, and ignored states with spaces and special characters.
        let mut raw = Vec::new();
        raw.extend_from_slice(b"1 MM N... 100644 100644 100644 1111111111111111111111111111111111111111 2222222222222222222222222222222222222222 src/lib.rs\0");
        raw.extend_from_slice(b"1 AM N... 100644 100755 100755 3333333333333333333333333333333333333333 4444444444444444444444444444444444444444 src/bin/cli.rs\0");
        raw.extend_from_slice(b"1 .U N... 100644 100644 100644 5555555555555555555555555555555555555555 6666666666666666666666666666666666666666 src/conflict.rs\0");
        raw.extend_from_slice(b"2 R. N... 100644 100644 100644 7777777777777777777777777777777777777777 8888888888888888888888888888888888888888 80 src/utils/helpers.rs\0old utils/helpers.rs\0");
        raw.extend_from_slice(b"2 C. N... 100644 100644 100644 9999999999999999999999999999999999999999 0000000000000000000000000000000000000000 60 scripts/setup.sh\0scripts/setup-old.sh\0");
        raw.extend_from_slice(b"1 D. N... 100644 000000 000000 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa 0000000000000000000000000000000000000000 docs/README.md\0");
        raw.extend_from_slice(b"1 A. N... 000000 100644 100644 0000000000000000000000000000000000000000 bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb \"space dir\"/new file.txt\0");
        raw.extend_from_slice(b"1 M. N... 100644 100644 100644 cccccccccccccccccccccccccccccccccccccccc dddddddddddddddddddddddddddddddddddddddd path/with->symbol.rs\0");
        raw.extend_from_slice(b"? assets/logo (1).svg\0");
        raw.extend_from_slice(b"? dir with spaces/file name [draft].md\0");
        raw.extend_from_slice(b"! target/.keep\0");
        raw.extend_from_slice(b"u UU N... 100644 100644 100644 eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee ffffffffffffffffffffffffffffffffffffffff 1 2 3 some unmerged/path.txt\0");

        let entries: Vec<StatusEntry> = parse_porcelain_v2(&raw).expect("parse succeeds");

        // High-level assertions about the parsed content
        assert_eq!(entries.len(), 12);
        assert!(
            entries
                .iter()
                .any(|e| e.path == "src/lib.rs" && e.staged == StatusCode::Modified)
        );
        assert!(entries.iter().any(|e| e.kind == EntryKind::Rename
            && e.orig_path.as_deref() == Some("old utils/helpers.rs")));
        assert!(
            entries.iter().any(|e| e.kind == EntryKind::Copy
                && e.orig_path.as_deref() == Some("scripts/setup-old.sh"))
        );
        assert!(entries.iter().any(|e| e.kind == EntryKind::Unmerged));
        assert!(
            entries
                .iter()
                .any(|e| matches!(e.unstaged, StatusCode::Untracked))
        );
        assert!(
            entries
                .iter()
                .any(|e| matches!(e.unstaged, StatusCode::Ignored))
        );

        assert_debug_snapshot!(entries);
    }
}
