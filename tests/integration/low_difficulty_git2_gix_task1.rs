use crate::repos::test_repo::TestRepo;
use git_ai::api::client::ApiContext;
use std::fs;
use std::process::Command;

fn write_file(repo: &TestRepo, path: &str, contents: &str) {
    let file_path = repo.path().join(path);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(file_path, contents).unwrap();
}


fn write_test_api_key_config(repo: &TestRepo, api_key: &str) {
    let config_dir = repo.test_home_path().join(".git-ai");
    fs::create_dir_all(&config_dir).expect("config dir should be creatable");
    fs::write(
        config_dir.join("config.json"),
        format!(r#"{{"api_key":"{}"}}"#, api_key),
    )
    .expect("config file should be writable");
}

fn run_identity_case(case: &str) {
    let test_name = match case {
        "config" => "low_difficulty_task1_resolve_git_identity_matches_git_var_config_identity_format",
        "env-overrides" => "low_difficulty_task1_resolve_git_identity_prefers_env_over_repo_config",
        "author-env-overrides" => "low_difficulty_task1_resolve_commit_author_identity_prefers_author_env_over_repo_config",
        "no-env-no-config" => "low_difficulty_task1_resolve_git_identity_returns_empty_without_env_or_config",
        "cache-semantics" => "low_difficulty_task1_identity_cache_semantics_are_stable",
        other => panic!("unknown identity case: {other}"),
    };
    let output = Command::new(std::env::current_exe().expect("current_exe should resolve"))
        .arg(test_name)
        .arg("--exact")
        .env("GIT_AI_TASK1_IDENTITY_CASE", case)
        .output()
        .expect("isolated child process should start");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "identity child case {case} should pass:\n{combined}");
}

#[test]
fn low_difficulty_task1_resolve_git_identity_matches_git_var_config_identity_format() {
    if std::env::var("GIT_AI_TASK1_IDENTITY_CASE").ok().as_deref() == Some("config") {
        run_identity_config_case();
        return;
    }
    run_identity_case("config");
}

fn run_identity_config_case() {
    let repo = TestRepo::new();
    let workdir = repo.path();
    write_test_api_key_config(&repo, "task1-key");
    let git_config_global = repo.test_home_path().join(".gitconfig");
    let xdg_config_home = repo.test_home_path().join(".config");
    let home = repo.test_home_path();

    unsafe {
        std::env::set_var("HOME", home);
        std::env::set_var("GIT_CONFIG_GLOBAL", git_config_global);
        std::env::set_var("XDG_CONFIG_HOME", xdg_config_home);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
    }
    std::env::set_current_dir(workdir).expect("should switch working directory");

    let expected = repo
        .git_with_env(
            &["var", "GIT_COMMITTER_IDENT"],
            &[("GIT_CONFIG_NOSYSTEM", "1")],
            Some(workdir),
        )
        .expect("git var should succeed");

    let actual = ApiContext::new(None)
        .author_identity
        .expect("ApiContext should expose configured git identity when api_key is present");

    let trimmed = expected.trim();
    assert!(trimmed.starts_with(&actual), "formatted identity should be the name/email prefix of git var output");
    assert!(trimmed.contains('<') && trimmed.contains('>'), "git var output should contain email formatting");
}

#[test]
fn low_difficulty_task1_resolve_git_identity_prefers_env_over_repo_config() {
    if std::env::var("GIT_AI_TASK1_IDENTITY_CASE").ok().as_deref() == Some("env-overrides") {
        run_identity_env_override_case();
        return;
    }
    run_identity_case("env-overrides");
}

fn run_identity_env_override_case() {
    let repo = TestRepo::new();
    write_file(&repo, "env.txt", "content\n");
    repo.stage_all_and_commit("initial").unwrap();
    repo.git(&["config", "user.name", "Repo User"]).unwrap();
    repo.git(&["config", "user.email", "repo@example.com"]).unwrap();
    write_test_api_key_config(&repo, "task1-key");

    let workdir = repo.path();
    let git_config_global = repo.test_home_path().join(".gitconfig");
    let xdg_config_home = repo.test_home_path().join(".config");
    let home = repo.test_home_path();
    unsafe {
        std::env::set_var("HOME", home);
        std::env::set_var("GIT_CONFIG_GLOBAL", git_config_global);
        std::env::set_var("XDG_CONFIG_HOME", xdg_config_home);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("GIT_AUTHOR_NAME", "Env User");
        std::env::set_var("GIT_AUTHOR_EMAIL", "env@example.com");
        std::env::set_var("GIT_COMMITTER_NAME", "Env User");
        std::env::set_var("GIT_COMMITTER_EMAIL", "env@example.com");
    }
    std::env::set_current_dir(workdir).expect("should switch working directory");

    let expected = repo
        .git_with_env(
            &["var", "GIT_COMMITTER_IDENT"],
            &[
                ("GIT_CONFIG_NOSYSTEM", "1"),
                ("GIT_AUTHOR_NAME", "Env User"),
                ("GIT_AUTHOR_EMAIL", "env@example.com"),
                ("GIT_COMMITTER_NAME", "Env User"),
                ("GIT_COMMITTER_EMAIL", "env@example.com"),
            ],
            Some(workdir),
        )
        .expect("git var should succeed with env override");

    let actual = ApiContext::new(None)
        .author_identity
        .expect("ApiContext should expose env-overridden identity when api_key is present");

    assert!(expected.trim().starts_with(&actual), "resolved identity should match the env-overridden git var output");
    assert_eq!(actual, "Env User <env@example.com>");
}

#[test]
fn low_difficulty_task1_resolve_commit_author_identity_prefers_author_env_over_repo_config() {
    if std::env::var("GIT_AI_TASK1_IDENTITY_CASE").ok().as_deref() == Some("author-env-overrides") {
        run_commit_author_env_override_case();
        return;
    }
    run_identity_case("author-env-overrides");
}

fn run_commit_author_env_override_case() {
    let repo = TestRepo::new();
    write_file(&repo, "author.txt", "content\n");
    repo.stage_all_and_commit("initial").unwrap();
    repo.git(&["config", "user.name", "Repo User"]).unwrap();
    repo.git(&["config", "user.email", "repo@example.com"]).unwrap();

    let workdir = repo.path();
    let git_config_global = repo.test_home_path().join(".gitconfig");
    let xdg_config_home = repo.test_home_path().join(".config");
    let home = repo.test_home_path();

    unsafe {
        std::env::set_var("HOME", home);
        std::env::set_var("GIT_CONFIG_GLOBAL", git_config_global);
        std::env::set_var("XDG_CONFIG_HOME", xdg_config_home);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("GIT_AUTHOR_NAME", "Author Env User");
        std::env::set_var("GIT_AUTHOR_EMAIL", "author-env@example.com");
        std::env::remove_var("GIT_COMMITTER_NAME");
        std::env::remove_var("GIT_COMMITTER_EMAIL");
    }
    std::env::set_current_dir(workdir).expect("should switch working directory");

    let repository = git_ai::git::repository::find_repository(&[])
        .expect("repository lookup should succeed");
    let actual = repository.git_commit_author_identity();

    assert_eq!(actual.formatted().as_deref(), Some("Author Env User <author-env@example.com>"));
}

#[test]
fn low_difficulty_task1_resolve_git_identity_returns_empty_without_env_or_config() {
    if std::env::var("GIT_AI_TASK1_IDENTITY_CASE").ok().as_deref() == Some("no-env-no-config") {
        run_no_env_no_config_case();
        return;
    }
    run_identity_case("no-env-no-config");
}

fn run_no_env_no_config_case() {
    let repo = TestRepo::new();
    write_file(&repo, "empty.txt", "content\n");
    repo.stage_all_and_commit("initial").unwrap();

    let workdir = repo.path();
    let git_config_global = repo.test_home_path().join(".gitconfig");
    let xdg_config_home = repo.test_home_path().join(".config");
    let home = repo.test_home_path();

    unsafe {
        std::env::set_var("HOME", home);
        std::env::set_var("GIT_CONFIG_GLOBAL", git_config_global);
        std::env::set_var("XDG_CONFIG_HOME", xdg_config_home);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::remove_var("GIT_AUTHOR_NAME");
        std::env::remove_var("GIT_AUTHOR_EMAIL");
        std::env::remove_var("GIT_COMMITTER_NAME");
        std::env::remove_var("GIT_COMMITTER_EMAIL");
        std::env::remove_var("EMAIL");
    }
    std::env::set_current_dir(workdir).expect("should switch working directory");

    let repository = git_ai::git::repository::find_repository(&[])
        .expect("repository lookup should succeed");

    let current_user = repository.git_author_identity();
    let commit_author = repository.git_commit_author_identity();

    assert_eq!(current_user.formatted(), None);
    assert_eq!(commit_author.formatted(), None);
}

#[test]
fn low_difficulty_task1_identity_cache_semantics_are_stable() {
    if std::env::var("GIT_AI_TASK1_IDENTITY_CASE").ok().as_deref() == Some("cache-semantics") {
        run_cache_semantics_case();
        return;
    }
    run_identity_case("cache-semantics");
}

fn run_cache_semantics_case() {
    let repo = TestRepo::new();
    write_file(&repo, "cache.txt", "content\n");
    repo.stage_all_and_commit("initial").unwrap();
    repo.git(&["config", "user.name", "Repo User"]).unwrap();
    repo.git(&["config", "user.email", "repo@example.com"]).unwrap();

    let workdir = repo.path();
    let git_config_global = repo.test_home_path().join(".gitconfig");
    let xdg_config_home = repo.test_home_path().join(".config");
    let home = repo.test_home_path();

    unsafe {
        std::env::set_var("HOME", home);
        std::env::set_var("GIT_CONFIG_GLOBAL", git_config_global);
        std::env::set_var("XDG_CONFIG_HOME", xdg_config_home);
        std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
        std::env::set_var("GIT_COMMITTER_NAME", "First Committer");
        std::env::set_var("GIT_COMMITTER_EMAIL", "first-committer@example.com");
        std::env::set_var("GIT_AUTHOR_NAME", "First Author");
        std::env::set_var("GIT_AUTHOR_EMAIL", "first-author@example.com");
    }
    std::env::set_current_dir(workdir).expect("should switch working directory");

    let repository = git_ai::git::repository::find_repository(&[])
        .expect("repository lookup should succeed");

    let cached_initial = repository.git_author_identity().formatted();
    let author_initial = repository.git_commit_author_identity().formatted();

    unsafe {
        std::env::set_var("GIT_COMMITTER_NAME", "Second Committer");
        std::env::set_var("GIT_COMMITTER_EMAIL", "second-committer@example.com");
        std::env::set_var("GIT_AUTHOR_NAME", "Second Author");
        std::env::set_var("GIT_AUTHOR_EMAIL", "second-author@example.com");
    }

    let cached_after_env_change = repository.git_author_identity().formatted();
    let author_after_env_change = repository.git_commit_author_identity().formatted();

    assert_eq!(cached_initial.as_deref(), Some("First Committer <first-committer@example.com>"));
    assert_eq!(cached_after_env_change.as_deref(), Some("First Committer <first-committer@example.com>"));
    assert_eq!(author_initial.as_deref(), Some("First Author <first-author@example.com>"));
    assert_eq!(author_after_env_change.as_deref(), Some("Second Author <second-author@example.com>"));
}
