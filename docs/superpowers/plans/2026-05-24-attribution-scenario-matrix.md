# Attribution Scenario Matrix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a broad attribution scenario matrix that creates multiple commits covering new files, edits, deletes, copies, and mixed human/AI flows, with both CI-grade assertions and a manual PowerShell reproducer.

**Architecture:** The Rust integration test is the source of truth and uses the existing `TestRepo`, `fs::write`, explicit `git-ai checkpoint mock_ai`, and `assert_lines_and_blame` helpers. The PowerShell script is a standalone manual reproducer that creates a disposable repository, runs the same scenario names, and prints content, `git-ai blame`, and AI note data after each commit.

**Tech Stack:** Rust integration tests, existing git-ai test harness, PowerShell 7+, Git CLI, git-ai CLI.

---

## File Structure

- Create `tests/integration/attribution_scenario_matrix.rs`
  - Owns one end-to-end Rust test that walks through a realistic multi-commit scenario matrix.
  - Uses raw `std::fs::write` instead of `TestFile::set_contents`, `insert_at`, or `replace_at` so checkpoint timing is explicit.
  - Asserts line-level attribution after each meaningful commit.
- Modify `tests/integration/main.rs`
  - Adds `mod attribution_scenario_matrix;` so the new test file is compiled by the integration target.
- Create `scripts/run-attribution-scenario-matrix.ps1`
  - Manual reproducer for local debugging.
  - Creates a temp repo, runs the scenario matrix with real `git-ai`, and prints each commit's blame/note output.

## Scenario Matrix

Use these scenario labels in both Rust and PowerShell:

1. `01_human_seed` - human creates initial tracked file.
2. `02_ai_new_file` - AI creates a new file and checkpoints it.
3. `03_ai_modifies_human_file` - AI modifies a human-authored file.
4. `04_uncheckpointed_human_modifies_ai` - human changes AI content without checkpoint.
5. `05_uncheckpointed_human_copies_ai` - human copies AI content without checkpoint.
6. `06_ai_deletes_human_line` - AI removes a human line and changes neighboring content.
7. `07_human_deletes_ai_line` - human removes an AI line without checkpoint.
8. `08_delete_and_recreate_file` - file is deleted and recreated with new AI content.
9. `09_mixed_multi_file_commit` - one commit contains checkpointed AI changes and uncheckpointed human changes across multiple files.

---

### Task 1: Add Rust Integration Matrix

**Files:**
- Create: `tests/integration/attribution_scenario_matrix.rs`
- Modify: `tests/integration/main.rs`

- [ ] **Step 1: Write the integration test file**

Create `tests/integration/attribution_scenario_matrix.rs` with this content:

```rust
use crate::repos::test_file::ExpectedLineExt;
use crate::repos::test_repo::TestRepo;
use std::fs;

fn write_file(repo: &TestRepo, rel: &str, content: &str) {
    fs::write(repo.path().join(rel), content).unwrap_or_else(|error| {
        panic!("failed writing {rel}: {error}");
    });
}

#[test]
fn attribution_scenario_matrix_tracks_new_modify_delete_copy_and_mixed_commits() {
    let repo = TestRepo::new();

    write_file(
        &repo,
        "app.rs",
        "fn seed() {\n    println!(\"human seed\");\n}\n",
    );
    repo.stage_all_and_commit("01_human_seed").unwrap();
    repo.filename("app.rs").assert_lines_and_blame(crate::lines![
        "fn seed() {".human(),
        "    println!(\"human seed\");".human(),
        "}".human(),
    ]);

    write_file(
        &repo,
        "generated.rs",
        "pub fn generated() {\n    println!(\"ai generated\");\n}\n",
    );
    repo.git_ai(&["checkpoint", "mock_ai", "generated.rs"]).unwrap();
    repo.stage_all_and_commit("02_ai_new_file").unwrap();
    repo.filename("generated.rs").assert_lines_and_blame(crate::lines![
        "pub fn generated() {".ai(),
        "    println!(\"ai generated\");".ai(),
        "}".ai(),
    ]);

    write_file(
        &repo,
        "app.rs",
        "fn seed() {\n    println!(\"ai updated human file\");\n}\n",
    );
    repo.git_ai(&["checkpoint", "mock_ai", "app.rs"]).unwrap();
    repo.stage_all_and_commit("03_ai_modifies_human_file").unwrap();
    repo.filename("app.rs").assert_lines_and_blame(crate::lines![
        "fn seed() {".human(),
        "    println!(\"ai updated human file\");".ai(),
        "}".human(),
    ]);

    write_file(
        &repo,
        "generated.rs",
        "pub fn generated() {\n    println!(\"human changed ai content\");\n}\n",
    );
    repo.stage_all_and_commit("04_uncheckpointed_human_modifies_ai").unwrap();
    repo.filename("generated.rs").assert_lines_and_blame(crate::lines![
        "pub fn generated() {".ai(),
        "    println!(\"human changed ai content\");".human(),
        "}".ai(),
    ]);

    write_file(
        &repo,
        "generated.rs",
        "pub fn generated() {\n    println!(\"human changed ai content\");\n}\npub fn copied() {\n    println!(\"human changed ai content\");\n}\n",
    );
    repo.stage_all_and_commit("05_uncheckpointed_human_copies_ai").unwrap();
    repo.filename("generated.rs").assert_lines_and_blame(crate::lines![
        "pub fn generated() {".ai(),
        "    println!(\"human changed ai content\");".human(),
        "}".ai(),
        "pub fn copied() {".human(),
        "    println!(\"human changed ai content\");".human(),
        "}".human(),
    ]);

    write_file(&repo, "human_delete_target.rs", "keep human\nremove human\n");
    repo.stage_all_and_commit("06a_seed_human_delete_target").unwrap();
    write_file(&repo, "human_delete_target.rs", "keep human with ai edit\n");
    repo.git_ai(&["checkpoint", "mock_ai", "human_delete_target.rs"]).unwrap();
    repo.stage_all_and_commit("06_ai_deletes_human_line").unwrap();
    repo.filename("human_delete_target.rs")
        .assert_lines_and_blame(crate::lines!["keep human with ai edit".ai()]);

    write_file(&repo, "generated.rs", "pub fn generated() {\n}\npub fn copied() {\n    println!(\"human changed ai content\");\n}\n");
    repo.stage_all_and_commit("07_human_deletes_ai_line").unwrap();
    repo.filename("generated.rs").assert_lines_and_blame(crate::lines![
        "pub fn generated() {".ai(),
        "}".ai(),
        "pub fn copied() {".human(),
        "    println!(\"human changed ai content\");".human(),
        "}".human(),
    ]);

    fs::remove_file(repo.path().join("generated.rs")).unwrap();
    repo.stage_all_and_commit("08a_delete_generated_file").unwrap();
    write_file(
        &repo,
        "generated.rs",
        "pub fn recreated() {\n    println!(\"new ai file\");\n}\n",
    );
    repo.git_ai(&["checkpoint", "mock_ai", "generated.rs"]).unwrap();
    repo.stage_all_and_commit("08_delete_and_recreate_file").unwrap();
    repo.filename("generated.rs").assert_lines_and_blame(crate::lines![
        "pub fn recreated() {".ai(),
        "    println!(\"new ai file\");".ai(),
        "}".ai(),
    ]);

    write_file(
        &repo,
        "generated.rs",
        "pub fn recreated() {\n    println!(\"new ai file\");\n}\npub fn mixed_ai() {\n    println!(\"checkpointed ai\");\n}\n",
    );
    repo.git_ai(&["checkpoint", "mock_ai", "generated.rs"]).unwrap();
    write_file(
        &repo,
        "app.rs",
        "fn seed() {\n    println!(\"ai updated human file\");\n}\nfn mixed_human() {\n    println!(\"uncheckpointed human\");\n}\n",
    );
    repo.stage_all_and_commit("09_mixed_multi_file_commit").unwrap();
    repo.filename("generated.rs").assert_lines_and_blame(crate::lines![
        "pub fn recreated() {".ai(),
        "    println!(\"new ai file\");".ai(),
        "}".ai(),
        "pub fn mixed_ai() {".ai(),
        "    println!(\"checkpointed ai\");".ai(),
        "}".ai(),
    ]);
    repo.filename("app.rs").assert_lines_and_blame(crate::lines![
        "fn seed() {".human(),
        "    println!(\"ai updated human file\");".ai(),
        "}".human(),
        "fn mixed_human() {".human(),
        "    println!(\"uncheckpointed human\");".human(),
        "}".human(),
    ]);
}
```

- [ ] **Step 2: Register the integration module**

Modify `tests/integration/main.rs` and add this line after `mod amp;`:

```rust
mod attribution_scenario_matrix;
```

The top of the module list should include:

```rust
mod agent_commits_blame;
mod agent_presets_comprehensive;
mod agent_v1;
mod ai_reflow_attribution;
mod ai_tab;
mod amend;
mod amp;
mod attribution_scenario_matrix;
mod attribution_tracker_comprehensive;
```

- [ ] **Step 3: Run the new integration test**

Run:

```powershell
cargo test --test integration attribution_scenario_matrix -- --nocapture
```

Expected:

```text
test attribution_scenario_matrix::attribution_scenario_matrix_tracks_new_modify_delete_copy_and_mixed_commits ... ok
```

If it fails because a line is unexpectedly AI or human, do not change the assertion first. Inspect `git-ai blame` output and decide whether the product expectation or implementation is wrong.

- [ ] **Step 4: Run diagnostics on changed Rust files**

Run LSP diagnostics for:

```text
tests/integration/attribution_scenario_matrix.rs
tests/integration/main.rs
```

Expected: no diagnostics.

---

### Task 2: Add PowerShell Reproducer Script

**Files:**
- Create: `scripts/run-attribution-scenario-matrix.ps1`

- [ ] **Step 1: Create the script**

Create `scripts/run-attribution-scenario-matrix.ps1` with this content:

```powershell
param(
    [string]$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path,
    [string]$WorkDir = "",
    [switch]$Keep
)

$ErrorActionPreference = "Stop"

function Invoke-LoggedCommand {
    param(
        [Parameter(Mandatory = $true)][string]$Label,
        [Parameter(Mandatory = $true)][string]$FilePath,
        [Parameter(Mandatory = $true)][string[]]$ArgumentList,
        [Parameter(Mandatory = $true)][string]$WorkingDirectory
    )

    "`n>>> $Label"
    "    $FilePath $($ArgumentList -join ' ')"
    Push-Location -LiteralPath $WorkingDirectory
    try {
        $output = & $FilePath @ArgumentList 2>&1
        $exitCode = $LASTEXITCODE
    } finally {
        Pop-Location
    }
    if ($output) {
        $output | ForEach-Object { "    $_" }
    }
    if ($exitCode -ne 0) {
        throw "Command failed ($exitCode): $Label"
    }
}

function Set-FileContentUtf8 {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$Content
    )
    [System.IO.File]::WriteAllText($Path, $Content, [System.Text.UTF8Encoding]::new($false))
}

function Show-AttributionState {
    param(
        [Parameter(Mandatory = $true)][string]$Scenario,
        [Parameter(Mandatory = $true)][string[]]$Files
    )

    "`n===== $Scenario ====="
    Invoke-LoggedCommand -Label "git rev-parse HEAD" -FilePath "git" -ArgumentList @("rev-parse", "HEAD") -WorkingDirectory $WorkDir

    foreach ($file in $Files) {
        if (Test-Path -LiteralPath (Join-Path $WorkDir $file)) {
            "`n--- content: $file ---"
            Get-Content -LiteralPath (Join-Path $WorkDir $file) | ForEach-Object { "    $_" }
            Invoke-LoggedCommand -Label "git-ai blame $file" -FilePath $GitAi -ArgumentList @("blame", $file) -WorkingDirectory $WorkDir
        } else {
            "`n--- content: $file deleted ---"
        }
    }

    "`n--- git notes --ref=ai show HEAD ---"
    $noteOutput = & git -C $WorkDir notes --ref=ai show HEAD 2>&1
    if ($LASTEXITCODE -eq 0) {
        $noteOutput | ForEach-Object { "    $_" }
    } else {
        "    <no ai note>"
    }
}

$GitAi = Join-Path $RepoRoot "target\debug\git-ai.exe"
if (-not (Test-Path -LiteralPath $GitAi)) {
    Invoke-LoggedCommand -Label "cargo build --bin git-ai --features test-support" -FilePath "cargo" -ArgumentList @("build", "--bin", "git-ai", "--features", "test-support") -WorkingDirectory $RepoRoot
}

if ([string]::IsNullOrWhiteSpace($WorkDir)) {
    $WorkDir = Join-Path ([System.IO.Path]::GetTempPath()) ("git-ai-attribution-matrix-" + [Guid]::NewGuid().ToString("N"))
}

New-Item -ItemType Directory -Path $WorkDir -Force | Out-Null
Invoke-LoggedCommand -Label "git init" -FilePath "git" -ArgumentList @("init") -WorkingDirectory $WorkDir
Invoke-LoggedCommand -Label "git config user.name" -FilePath "git" -ArgumentList @("config", "user.name", "Matrix Human") -WorkingDirectory $WorkDir
Invoke-LoggedCommand -Label "git config user.email" -FilePath "git" -ArgumentList @("config", "user.email", "matrix-human@example.com") -WorkingDirectory $WorkDir

try {
    Set-FileContentUtf8 -Path (Join-Path $WorkDir "app.rs") -Content ((@('fn seed() {', '    println!("human seed");', '}') -join "`n") + "`n")
    Invoke-LoggedCommand -Label "commit 01_human_seed" -FilePath "git" -ArgumentList @("add", ".") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 01_human_seed" -FilePath "git" -ArgumentList @("commit", "-m", "01_human_seed") -WorkingDirectory $WorkDir
    Show-AttributionState -Scenario "01_human_seed" -Files @("app.rs")

    Set-FileContentUtf8 -Path (Join-Path $WorkDir "generated.rs") -Content ((@('pub fn generated() {', '    println!("ai generated");', '}') -join "`n") + "`n")
    Invoke-LoggedCommand -Label "checkpoint 02_ai_new_file" -FilePath $GitAi -ArgumentList @("checkpoint", "mock_ai", "generated.rs") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 02_ai_new_file add" -FilePath "git" -ArgumentList @("add", ".") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 02_ai_new_file" -FilePath "git" -ArgumentList @("commit", "-m", "02_ai_new_file") -WorkingDirectory $WorkDir
    Show-AttributionState -Scenario "02_ai_new_file" -Files @("generated.rs")

    Set-FileContentUtf8 -Path (Join-Path $WorkDir "app.rs") -Content ((@('fn seed() {', '    println!("ai updated human file");', '}') -join "`n") + "`n")
    Invoke-LoggedCommand -Label "checkpoint 03_ai_modifies_human_file" -FilePath $GitAi -ArgumentList @("checkpoint", "mock_ai", "app.rs") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 03_ai_modifies_human_file add" -FilePath "git" -ArgumentList @("add", ".") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 03_ai_modifies_human_file" -FilePath "git" -ArgumentList @("commit", "-m", "03_ai_modifies_human_file") -WorkingDirectory $WorkDir
    Show-AttributionState -Scenario "03_ai_modifies_human_file" -Files @("app.rs")

    Set-FileContentUtf8 -Path (Join-Path $WorkDir "generated.rs") -Content ((@('pub fn generated() {', '    println!("human changed ai content");', '}') -join "`n") + "`n")
    Invoke-LoggedCommand -Label "commit 04_uncheckpointed_human_modifies_ai add" -FilePath "git" -ArgumentList @("add", ".") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 04_uncheckpointed_human_modifies_ai" -FilePath "git" -ArgumentList @("commit", "-m", "04_uncheckpointed_human_modifies_ai") -WorkingDirectory $WorkDir
    Show-AttributionState -Scenario "04_uncheckpointed_human_modifies_ai" -Files @("generated.rs")

    Set-FileContentUtf8 -Path (Join-Path $WorkDir "generated.rs") -Content ((@('pub fn generated() {', '    println!("human changed ai content");', '}', 'pub fn copied() {', '    println!("human changed ai content");', '}') -join "`n") + "`n")
    Invoke-LoggedCommand -Label "commit 05_uncheckpointed_human_copies_ai add" -FilePath "git" -ArgumentList @("add", ".") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 05_uncheckpointed_human_copies_ai" -FilePath "git" -ArgumentList @("commit", "-m", "05_uncheckpointed_human_copies_ai") -WorkingDirectory $WorkDir
    Show-AttributionState -Scenario "05_uncheckpointed_human_copies_ai" -Files @("generated.rs")

    Set-FileContentUtf8 -Path (Join-Path $WorkDir "human_delete_target.rs") -Content "keep human`nremove human`n"
    Invoke-LoggedCommand -Label "commit 06a_seed_human_delete_target add" -FilePath "git" -ArgumentList @("add", ".") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 06a_seed_human_delete_target" -FilePath "git" -ArgumentList @("commit", "-m", "06a_seed_human_delete_target") -WorkingDirectory $WorkDir
    Set-FileContentUtf8 -Path (Join-Path $WorkDir "human_delete_target.rs") -Content "keep human with ai edit`n"
    Invoke-LoggedCommand -Label "checkpoint 06_ai_deletes_human_line" -FilePath $GitAi -ArgumentList @("checkpoint", "mock_ai", "human_delete_target.rs") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 06_ai_deletes_human_line add" -FilePath "git" -ArgumentList @("add", ".") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 06_ai_deletes_human_line" -FilePath "git" -ArgumentList @("commit", "-m", "06_ai_deletes_human_line") -WorkingDirectory $WorkDir
    Show-AttributionState -Scenario "06_ai_deletes_human_line" -Files @("human_delete_target.rs")

    Set-FileContentUtf8 -Path (Join-Path $WorkDir "generated.rs") -Content ((@('pub fn generated() {', '}', 'pub fn copied() {', '    println!("human changed ai content");', '}') -join "`n") + "`n")
    Invoke-LoggedCommand -Label "commit 07_human_deletes_ai_line add" -FilePath "git" -ArgumentList @("add", ".") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 07_human_deletes_ai_line" -FilePath "git" -ArgumentList @("commit", "-m", "07_human_deletes_ai_line") -WorkingDirectory $WorkDir
    Show-AttributionState -Scenario "07_human_deletes_ai_line" -Files @("generated.rs")

    Remove-Item -LiteralPath (Join-Path $WorkDir "generated.rs")
    Invoke-LoggedCommand -Label "commit 08a_delete_generated_file add" -FilePath "git" -ArgumentList @("add", ".") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 08a_delete_generated_file" -FilePath "git" -ArgumentList @("commit", "-m", "08a_delete_generated_file") -WorkingDirectory $WorkDir
    Set-FileContentUtf8 -Path (Join-Path $WorkDir "generated.rs") -Content ((@('pub fn recreated() {', '    println!("new ai file");', '}') -join "`n") + "`n")
    Invoke-LoggedCommand -Label "checkpoint 08_delete_and_recreate_file" -FilePath $GitAi -ArgumentList @("checkpoint", "mock_ai", "generated.rs") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 08_delete_and_recreate_file add" -FilePath "git" -ArgumentList @("add", ".") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 08_delete_and_recreate_file" -FilePath "git" -ArgumentList @("commit", "-m", "08_delete_and_recreate_file") -WorkingDirectory $WorkDir
    Show-AttributionState -Scenario "08_delete_and_recreate_file" -Files @("generated.rs")

    Set-FileContentUtf8 -Path (Join-Path $WorkDir "generated.rs") -Content ((@('pub fn recreated() {', '    println!("new ai file");', '}', 'pub fn mixed_ai() {', '    println!("checkpointed ai");', '}') -join "`n") + "`n")
    Invoke-LoggedCommand -Label "checkpoint 09_mixed_multi_file_commit" -FilePath $GitAi -ArgumentList @("checkpoint", "mock_ai", "generated.rs") -WorkingDirectory $WorkDir
    Set-FileContentUtf8 -Path (Join-Path $WorkDir "app.rs") -Content ((@('fn seed() {', '    println!("ai updated human file");', '}', 'fn mixed_human() {', '    println!("uncheckpointed human");', '}') -join "`n") + "`n")
    Invoke-LoggedCommand -Label "commit 09_mixed_multi_file_commit add" -FilePath "git" -ArgumentList @("add", ".") -WorkingDirectory $WorkDir
    Invoke-LoggedCommand -Label "commit 09_mixed_multi_file_commit" -FilePath "git" -ArgumentList @("commit", "-m", "09_mixed_multi_file_commit") -WorkingDirectory $WorkDir
    Show-AttributionState -Scenario "09_mixed_multi_file_commit" -Files @("generated.rs", "app.rs")

    "`nScenario matrix complete. Repo: $WorkDir"
} finally {
    if (-not $Keep) {
        "`nCleaning up $WorkDir. Re-run with -Keep to inspect the repository."
        Remove-Item -LiteralPath $WorkDir -Recurse -Force -ErrorAction SilentlyContinue
    }
}
```

- [ ] **Step 2: Run the reproducer with a kept temp repo**

Run:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/run-attribution-scenario-matrix.ps1 -Keep
```

Expected:

```text
===== 01_human_seed =====
...
===== 09_mixed_multi_file_commit =====
...
Scenario matrix complete. Repo: <temp path>
```

If the script fails because `git-ai.exe` does not exist, verify that the script ran `cargo build --bin git-ai --features test-support` and inspect the build error.

- [ ] **Step 3: Run script cleanup mode**

Run:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/run-attribution-scenario-matrix.ps1
```

Expected:

```text
Scenario matrix complete. Repo: <temp path>
Cleaning up <temp path>. Re-run with -Keep to inspect the repository.
```

---

### Task 3: Verification and Cleanup

**Files:**
- Verify: `tests/integration/attribution_scenario_matrix.rs`
- Verify: `tests/integration/main.rs`
- Verify: `scripts/run-attribution-scenario-matrix.ps1`

- [ ] **Step 1: Run targeted Rust test**

Run:

```powershell
cargo test --test integration attribution_scenario_matrix -- --nocapture
```

Expected:

```text
test attribution_scenario_matrix::attribution_scenario_matrix_tracks_new_modify_delete_copy_and_mixed_commits ... ok
```

- [ ] **Step 2: Run script smoke test**

Run:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/run-attribution-scenario-matrix.ps1
```

Expected: script exits 0 and prints `Scenario matrix complete`.

- [ ] **Step 3: Run diagnostics**

Run LSP diagnostics on:

```text
tests/integration/attribution_scenario_matrix.rs
tests/integration/main.rs
```

Expected: no diagnostics.

- [ ] **Step 4: Inspect final diff**

Run:

```powershell
git diff -- tests/integration/attribution_scenario_matrix.rs tests/integration/main.rs scripts/run-attribution-scenario-matrix.ps1
```

Expected: only the new integration matrix, `mod attribution_scenario_matrix;`, and the PowerShell reproducer are present.

- [ ] **Step 5: Report verification caveats**

In the final response, include:

```text
Verified:
- cargo test --test integration attribution_scenario_matrix -- --nocapture
- pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/run-attribution-scenario-matrix.ps1
- LSP diagnostics on changed Rust files

Notes:
- The PowerShell script creates and deletes a temp repo by default; use -Keep to inspect it.
- Existing unrelated dirty files were left untouched.
```

---

## Self-Review Checklist

- Spec coverage: Tasks cover the Rust CI regression, PowerShell manual reproducer, scenario names, temp repo behavior, blame/note output, and verification.
- Placeholder scan: No placeholders remain; every file has exact content or exact edit instructions.
- Type consistency: Rust uses existing `TestRepo`, `ExpectedLineExt`, `repo.git_ai`, `repo.stage_all_and_commit`, and `assert_lines_and_blame` APIs already used by the integration suite.
- Scope control: No production attribution logic is changed by this plan; it only adds regression/diagnostic tooling.
