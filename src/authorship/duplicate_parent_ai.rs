use crate::authorship::authorship_log_serialization::AuthorshipLog;
use crate::git::repository::Repository;
use std::collections::{HashMap, HashSet};

pub(crate) struct DuplicateParentAiContext<'a> {
    repo: &'a Repository,
    commit_sha: &'a str,
    parent_sha: Option<&'a str>,
    parent_authorship_log: Option<&'a AuthorshipLog>,
    current_file_lines: HashMap<String, Option<Vec<String>>>,
    parent_file_lines: HashMap<String, Option<Vec<String>>>,
    parent_prompt_lines: HashMap<(String, String), HashSet<String>>,
}

impl<'a> DuplicateParentAiContext<'a> {
    pub(crate) fn new_exact(
        repo: &'a Repository,
        commit_sha: &'a str,
        parent_sha: Option<&'a str>,
        parent_authorship_log: Option<&'a AuthorshipLog>,
    ) -> Self {
        Self {
            repo,
            commit_sha,
            parent_sha,
            parent_authorship_log,
            current_file_lines: HashMap::new(),
            parent_file_lines: HashMap::new(),
            parent_prompt_lines: HashMap::new(),
        }
    }

    pub(crate) fn is_duplicate_parent_ai_line(
        &mut self,
        file_path: &str,
        current_line: u32,
        prompt_hash: &str,
    ) -> bool {
        let Some(line_text) = self.current_line_text(file_path, current_line) else {
            return false;
        };
        if line_text.is_empty() {
            return false;
        }

        self.parent_prompt_line_texts(file_path, prompt_hash)
            .contains(&line_text)
    }

    fn current_line_text(&mut self, file_path: &str, line: u32) -> Option<String> {
        let commit_sha = self.commit_sha;
        let repo = self.repo;
        let lines = self
            .current_file_lines
            .entry(file_path.to_string())
            .or_insert_with(|| read_lines_at_commit(repo, commit_sha, file_path));
        line.checked_sub(1)
            .and_then(|idx| lines.as_ref()?.get(idx as usize).cloned())
    }

    fn parent_prompt_line_texts(&mut self, file_path: &str, prompt_hash: &str) -> &HashSet<String> {
        let key = (file_path.to_string(), prompt_hash.to_string());
        if !self.parent_prompt_lines.contains_key(&key) {
            let lines = self.collect_parent_prompt_line_texts(file_path, prompt_hash);
            self.parent_prompt_lines.insert(key.clone(), lines);
        }
        self.parent_prompt_lines
            .get(&key)
            .expect("parent prompt line cache should contain key")
    }

    fn collect_parent_prompt_line_texts(
        &mut self,
        file_path: &str,
        prompt_hash: &str,
    ) -> HashSet<String> {
        let Some(parent_sha) = self.parent_sha else {
            return HashSet::new();
        };
        let Some(parent_log) = self.parent_authorship_log else {
            return HashSet::new();
        };

        let repo = self.repo;
        let parent_lines = self
            .parent_file_lines
            .entry(file_path.to_string())
            .or_insert_with(|| read_lines_at_commit(repo, parent_sha, file_path));
        let Some(parent_lines) = parent_lines.as_ref() else {
            return HashSet::new();
        };

        let mut texts = HashSet::new();
        for file_attestation in &parent_log.attestations {
            if file_attestation.file_path != file_path {
                continue;
            }
            for entry in &file_attestation.entries {
                if entry.hash != prompt_hash {
                    continue;
                }
                for range in &entry.line_ranges {
                    for line in range.expand() {
                        let Some(text) = line
                            .checked_sub(1)
                            .and_then(|idx| parent_lines.get(idx as usize))
                        else {
                            continue;
                        };
                        if !text.is_empty() {
                            texts.insert(text.clone());
                        }
                    }
                }
            }
        }
        texts
    }
}

fn read_lines_at_commit(
    repo: &Repository,
    commit_sha: &str,
    file_path: &str,
) -> Option<Vec<String>> {
    let bytes = repo.get_file_content(file_path, commit_sha).ok()?;
    let content = String::from_utf8_lossy(&bytes);
    Some(content.lines().map(|line| line.to_string()).collect())
}
