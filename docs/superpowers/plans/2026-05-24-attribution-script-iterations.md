# Attribution Script Iterations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the manual attribution PowerShell script with more scenarios and safe 100-iteration execution.

**Architecture:** Keep one script, but refactor the scenario body into a reusable suite function. Each iteration creates a fresh repository, isolated HOME/config, and isolated test DB so attribution state and daemon state cannot leak between runs.

**Tech Stack:** PowerShell 7, real Git executable, local `target/debug/git-ai.exe`, disposable Git repositories.

---

## File Structure

- Modify: `scripts/run-attribution-first-commit-scenarios.ps1`
  - Add `-Iterations` and `-VerboseOutput` parameters.
  - Add per-iteration repo setup and cleanup.
  - Preserve real Git path enforcement.
  - Add five new scenarios.
  - Make normal output compact and detailed output opt-in.

## Task 1: Add Loop Parameters and Output Control

**Files:**
- Modify: `scripts/run-attribution-first-commit-scenarios.ps1:1-7`

- [ ] **Step 1: Extend parameters**

Add these parameters:

```powershell
[int]$Iterations = 1,
[switch]$VerboseOutput
```

Validate immediately after strict mode:

```powershell
if ($Iterations -lt 1) {
    throw "-Iterations must be at least 1."
}
```

- [ ] **Step 2: Gate detailed output**

Update `Show-AttributionState` so it always returns blame/note data but only prints detailed file, blame, and note output when `$VerboseOutput` is set.

Expected compact output per iteration:

```text
[1/100] passed
```

## Task 2: Refactor Into Per-Iteration Execution

**Files:**
- Modify: `scripts/run-attribution-first-commit-scenarios.ps1:290-398`

- [ ] **Step 1: Extract repo initialization**

Create `Initialize-TestRepo` that accepts an iteration number and returns an object with:

```powershell
@{
    RepoPath = $repoPath
    TestHome = $testHome
    TestDbPath = $testDbPath
    CreatedTemp = $createdTemp
}
```

The function must create a fresh temp repo by default for every iteration.

- [ ] **Step 2: Extract scenario suite**

Create `Run-ScenarioSuite` with parameters:

```powershell
param([string]$RepoPath)
```

Move all existing scenario code into it.

- [ ] **Step 3: Add iteration loop**

Add a loop:

```powershell
for ($iteration = 1; $iteration -le $Iterations; $iteration++) {
    ...
}
```

Each iteration initializes a repo, runs `Run-ScenarioSuite`, prints `[N/M] passed`, and cleans up unless `-Keep` is set.

If a failure occurs, keep that iteration's repo even when `-Keep` is not set, then rethrow.

## Task 3: Add Five More Scenarios

**Files:**
- Modify: `scripts/run-attribution-first-commit-scenarios.ps1` inside `Run-ScenarioSuite`

- [ ] **Step 1: Add scenario 06, AI modifies human file**

Create `existing-human.txt` as human, commit it, then AI changes one line. Assert modified line is `mock_ai`, untouched lines are not.

- [ ] **Step 2: Add scenario 07, human modifies AI line**

Create `human-overrides-ai.txt` as AI and commit it. Rewrite one AI line without a human checkpoint and commit. Assert rewritten line is not `mock_ai`, surrounding AI lines remain `mock_ai`.

- [ ] **Step 3: Add scenario 08, human deletes AI line**

Create three AI lines, commit them, remove the middle line without checkpoint, commit. Assert remaining lines are still `mock_ai` and the deleted text is absent from blame.

- [ ] **Step 4: Add scenario 09, AI reflows one human line**

Create one human line `call(foo, bar, baz)`, commit it, rewrite as five AI lines and checkpoint. Assert all reflowed lines are `mock_ai`.

- [ ] **Step 5: Add scenario 10, multi-file scoped checkpoint**

Create base files `scoped-ai.txt` and `unscoped-human.txt`, commit as human. Add a line to both, checkpoint only `scoped-ai.txt`, commit. Assert scoped new line is `mock_ai` and unscoped new line is not.

## Task 4: Verify Script

**Files:**
- Verify: `scripts/run-attribution-first-commit-scenarios.ps1`

- [ ] **Step 1: Parse syntax**

Run:

```powershell
pwsh -NoProfile -Command '$errors = $null; [System.Management.Automation.Language.Parser]::ParseFile("scripts/run-attribution-first-commit-scenarios.ps1", [ref]$null, [ref]$errors) | Out-Null; if ($errors.Count -gt 0) { $errors | ForEach-Object { $_.ToString() }; exit 1 }'
```

Expected: no output, exit 0.

- [ ] **Step 2: Run one compact iteration**

Run:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/run-attribution-first-commit-scenarios.ps1 -SkipBuild -GitPath "C:\Program Files\Git\cmd\git.exe" -Iterations 1
```

Expected: `[1/1] passed` and `All manual attribution scenario iterations passed.`

- [ ] **Step 3: Run 100 compact iterations**

Run:

```powershell
pwsh -NoProfile -ExecutionPolicy Bypass -File scripts/run-attribution-first-commit-scenarios.ps1 -SkipBuild -GitPath "C:\Program Files\Git\cmd\git.exe" -Iterations 100
```

Expected: `[100/100] passed` and `All manual attribution scenario iterations passed.`

## Self-Review

- Spec coverage: adds scenarios from existing-file edits, deletion, formatting/reflow, and multi-file scoped checkpoint categories.
- Placeholder scan: no placeholders intended.
- Type consistency: functions use PowerShell strings and hashtables already present in the script.
