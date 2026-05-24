# git-ai AI Code Attribution Manual Test Report

## Test Objective

This test verifies whether `git-ai` correctly records line-level attribution across mixed AI and human editing flows.

The primary focus is ensuring that human-created, human-copied, and human-modified code is not incorrectly attributed to AI, even when the text originated from nearby AI-generated code.

The test also verifies that repeated local execution does not trigger git-ai wrapper or daemon recursion.

## Test Script

```powershell
scripts/run-attribution-first-commit-scenarios.ps1
```

The script uses the real Git executable explicitly:

```powershell
-GitPath "C:\Program Files\Git\cmd\git.exe"
```

This avoids routing plain Git commands through:

```text
C:\Users\tbano\.git-ai\bin\git.exe
```

That safeguard prevents git-ai wrapper or daemon recursion during stress runs.

## Environment

- Platform: Windows
- Shell: PowerShell 7
- git-ai binary: `target\debug\git-ai.exe`
- Git executable: `C:\Program Files\Git\cmd\git.exe`
- Each iteration uses:
  - a fresh temporary Git repository
  - isolated `HOME`, `USERPROFILE`, `APPDATA`, and `XDG_CONFIG_HOME`
  - an isolated `GIT_AI_TEST_DB_PATH`
  - `GIT_AI_DAEMON_CHECKPOINT_DELEGATE=false`
- Default output is compact. Detailed blame and note output is available with `-VerboseOutput`.

## Commands Run

PowerShell parser check:

```powershell
pwsh -NoProfile -Command '$errors = $null; [System.Management.Automation.Language.Parser]::ParseFile("scripts/run-attribution-first-commit-scenarios.ps1", [ref]$null, [ref]$errors) | Out-Null; if ($errors.Count -gt 0) { $errors | ForEach-Object { $_.ToString() }; exit 1 }'
```

Single iteration:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/run-attribution-first-commit-scenarios.ps1 -SkipBuild -GitPath "C:\Program Files\Git\cmd\git.exe" -Iterations 1
```

100-iteration stress run:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/run-attribution-first-commit-scenarios.ps1 -SkipBuild -GitPath "C:\Program Files\Git\cmd\git.exe" -Iterations 100
```

## Results

### Parser Check

Result: passed.

The parser check produced no errors.

### Single Iteration

Result: passed.

Observed output:

```text
[1/1] passed
All manual attribution scenario iterations passed.
```

### 100-Iteration Stress Run

Result: passed.

Observed output ended with:

```text
[100/100] passed
All manual attribution scenario iterations passed.
```

The 100-iteration run completed without assertion failures and without the previous daemon or wrapper process explosion.

## Scenario Coverage

The script currently covers 14 scenarios.

### Initial Commit and Mixed Pre-Commit Attribution

1. `01_ai_creates_new_file`
   - AI creates a new file.
   - Expected: generated lines are attributed to `mock_ai`.
   - Result: passed.

2. `02_human_creates_new_file_ai_appends_before_commit`
   - Human creates a file, then AI appends before the first commit.
   - Expected: human seed lines are not `mock_ai`; AI appended lines are `mock_ai`.
   - Result: passed.

3. `03_ai_creates_new_file_known_human_appends_before_commit`
   - AI creates a file, then known human appends before the first commit.
   - Expected: AI seed lines are `mock_ai`; human appended lines are not `mock_ai`.
   - Result: passed.

4. `04_ai_creates_new_file_legacy_human_then_ai`
   - AI writes, legacy human writes, then AI writes again.
   - Expected: AI lines are `mock_ai`; legacy human line is not `mock_ai`.
   - Result: passed.

5. `05_known_human_inserts_between_ai_lines_before_first_commit`
   - Known human inserts between two AI lines before the first commit.
   - Expected: surrounding lines are `mock_ai`; inserted line is not `mock_ai`.
   - Result: passed.

### Existing File, Deletion, Reflow, and Scoped Checkpoint Behavior

6. `06_ai_modifies_existing_human_file`
   - AI modifies an existing human-authored file.
   - Expected: modified line is `mock_ai`; untouched human lines are not `mock_ai`.
   - Result: passed.

7. `07_human_modifies_committed_ai_line`
   - Human modifies a committed AI line.
   - Expected: unchanged AI lines remain `mock_ai`; modified human line is not `mock_ai`.
   - Result: passed.

8. `08_human_deletes_ai_line`
   - Human deletes one AI line.
   - Expected: deleted line is absent from blame; remaining AI lines stay `mock_ai`.
   - Result: passed.

9. `09_ai_reflows_one_human_line`
   - AI reflows one human line into multiple lines.
   - Expected: reflowed lines are `mock_ai`.
   - Result: passed.

10. `10_multi_file_scoped_checkpoint`
    - Two files are changed, but only one file is included in the AI checkpoint.
    - Expected: scoped file's new line is `mock_ai`; unscoped file's new line is not `mock_ai`.
    - Result: passed.

### Human Copy and Human Modification of AI-Origin Text

11. `11_before_first_commit_human_copies_ai_lines`
    - AI generates lines, then human copies those lines before the first commit.
    - Expected: original AI lines are `mock_ai`; human-copied lines are not `mock_ai`.
    - Result: passed.

12. `12_committed_ai_human_copies_ai_lines`
    - AI-generated lines are committed, then human copies those lines in a later commit.
    - Expected: original committed AI lines remain `mock_ai`; human-copied lines are not `mock_ai`.
    - Result: passed.

13. `13_before_first_commit_human_adds_and_modifies_ai_code`
    - AI generates code, then human modifies one AI line and adds one line before the first commit.
    - Expected: unchanged AI lines are `mock_ai`; human-modified and human-added lines are not `mock_ai`.
    - Result: passed.

14. `14_committed_ai_human_modifies_ai_code`
    - AI-generated code is committed, then human modifies one AI-generated line in a later commit.
    - Expected: unchanged AI lines remain `mock_ai`; human-modified line is not `mock_ai`.
    - Result: passed.

## Conclusions

- `git-ai` correctly distinguishes AI-generated code from human-copied or human-modified AI-origin text in the tested scenarios.
- The four suspected failure modes involving human copy or human modification of AI code did not reproduce with the current implementation.
- All 14 scenarios passed in a single iteration.
- All 14 scenarios passed across 100 fresh-repository iterations.
- The real-Git-path safeguard prevented the previous git-ai wrapper or daemon recursion problem during stress execution.

## Notes and Limitations

- This is a manual PowerShell scenario script, not a Rust integration test.
- The script exercises `mock_ai`, `mock_known_human`, and legacy `human` checkpoint flows.
- It does not directly cover real agent preset hook inputs such as Claude, Cursor, OpenCode, or Windsurf.
- For CI-grade protection, the highest-risk scenarios should be ported into Rust integration tests.

## Files Involved

- Script: `scripts/run-attribution-first-commit-scenarios.ps1`
- Report: `docs/superpowers/reports/2026-05-24-git-ai-attribution-manual-test-report.md`
