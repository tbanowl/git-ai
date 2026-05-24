param(
    [string]$RepoRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot "..")).Path,
    [string]$GitPath = "",
    [string]$WorkDir = "",
    [int]$Iterations = 1,
    [switch]$VerboseOutput,
    [switch]$Keep,
    [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

if ($Iterations -lt 1) {
    throw "-Iterations must be at least 1."
}

$script:CommonEnv = @{}

function Find-RealGitPath {
    param([string]$PreferredGitPath)

    if (-not [string]::IsNullOrWhiteSpace($PreferredGitPath)) {
        $resolved = (Resolve-Path -LiteralPath $PreferredGitPath).Path
        if ($resolved -match "(?i)\\\.git-ai\\|git-ai") {
            throw "-GitPath must point to the real Git executable, not a git-ai wrapper: $resolved"
        }
        return $resolved
    }

    $candidates = @(& where.exe git 2>$null) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
    foreach ($candidate in $candidates) {
        $resolved = (Resolve-Path -LiteralPath $candidate).Path
        if ($resolved -notmatch "(?i)\\\.git-ai\\|git-ai") {
            return $resolved
        }
    }

    throw "Could not find a real Git executable. Pass -GitPath 'C:\Program Files\Git\cmd\git.exe'. Candidates were: $($candidates -join ', ')"
}

function Invoke-CheckedCommand {
    param(
        [Parameter(Mandatory = $true)] [string]$Label,
        [Parameter(Mandatory = $true)] [string]$FilePath,
        [Parameter(Mandatory = $true)] [string[]]$ArgumentList,
        [Parameter(Mandatory = $true)] [string]$WorkingDirectory,
        [hashtable]$Environment = @{},
        [switch]$AllowFailure
    )

    $startInfo = [System.Diagnostics.ProcessStartInfo]::new()
    $startInfo.FileName = $FilePath
    foreach ($argument in $ArgumentList) {
        [void]$startInfo.ArgumentList.Add($argument)
    }
    $startInfo.WorkingDirectory = $WorkingDirectory
    $startInfo.RedirectStandardOutput = $true
    $startInfo.RedirectStandardError = $true
    $startInfo.UseShellExecute = $false

    foreach ($key in $script:CommonEnv.Keys) {
        $startInfo.Environment[$key] = [string]$script:CommonEnv[$key]
    }
    foreach ($key in $Environment.Keys) {
        $startInfo.Environment[$key] = [string]$Environment[$key]
    }

    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $startInfo
    [void]$process.Start()
    $stdout = $process.StandardOutput.ReadToEnd()
    $stderr = $process.StandardError.ReadToEnd()
    $process.WaitForExit()

    $output = @()
    if ($stdout.Length -gt 0) {
        $output += ($stdout -split "`r?`n" | Where-Object { $_ -ne "" })
    }
    if ($stderr.Length -gt 0) {
        $output += ($stderr -split "`r?`n" | Where-Object { $_ -ne "" })
    }

    if ($process.ExitCode -ne 0 -and -not $AllowFailure) {
        $displayArgs = $ArgumentList -join " "
        throw "Command failed during ${Label}: $FilePath $displayArgs`nExit code: $($process.ExitCode)`n$output"
    }

    return [pscustomobject]@{ ExitCode = $process.ExitCode; Output = [string[]]$output }
}

function Invoke-Git {
    param(
        [Parameter(Mandatory = $true)] [string]$Label,
        [Parameter(Mandatory = $true)] [string[]]$ArgumentList,
        [Parameter(Mandatory = $true)] [string]$WorkingDirectory,
        [switch]$AllowFailure
    )

    return Invoke-CheckedCommand -Label $Label -FilePath $script:RealGitPath -ArgumentList $ArgumentList -WorkingDirectory $WorkingDirectory -AllowFailure:$AllowFailure
}

function Invoke-GitAi {
    param(
        [Parameter(Mandatory = $true)] [string]$Label,
        [Parameter(Mandatory = $true)] [string[]]$ArgumentList,
        [Parameter(Mandatory = $true)] [string]$WorkingDirectory,
        [switch]$AsGitProxy,
        [switch]$AllowFailure
    )

    $envVars = @{}
    if ($AsGitProxy) {
        $envVars["GIT_AI"] = "git"
    }

    return Invoke-CheckedCommand -Label $Label -FilePath $script:GitAiPath -ArgumentList $ArgumentList -WorkingDirectory $WorkingDirectory -Environment $envVars -AllowFailure:$AllowFailure
}

function Set-FileContentUtf8 {
    param(
        [Parameter(Mandatory = $true)] [string]$RepoPath,
        [Parameter(Mandatory = $true)] [string]$RelativePath,
        [Parameter(Mandatory = $true)] [string]$Content
    )

    $path = Join-Path $RepoPath $RelativePath
    $parent = Split-Path -Parent $path
    if ($parent -and -not (Test-Path -LiteralPath $parent)) {
        New-Item -ItemType Directory -Path $parent | Out-Null
    }
    [System.IO.File]::WriteAllText($path, $Content, [System.Text.UTF8Encoding]::new($false))
}

function Commit-All {
    param([string]$RepoPath, [string]$Message)
    Invoke-GitAi -Label "git add for $Message" -ArgumentList @("add", ".") -WorkingDirectory $RepoPath -AsGitProxy | Out-Null
    Invoke-GitAi -Label "git commit for $Message" -ArgumentList @("commit", "-m", $Message) -WorkingDirectory $RepoPath -AsGitProxy | Out-Null
}

function Checkpoint-Ai { param([string]$RepoPath, [string]$RelativePath) Invoke-GitAi -Label "AI checkpoint for $RelativePath" -ArgumentList @("checkpoint", "mock_ai", $RelativePath) -WorkingDirectory $RepoPath | Out-Null }
function Checkpoint-KnownHuman { param([string]$RepoPath, [string]$RelativePath) Invoke-GitAi -Label "known human checkpoint for $RelativePath" -ArgumentList @("checkpoint", "mock_known_human", $RelativePath) -WorkingDirectory $RepoPath | Out-Null }
function Checkpoint-LegacyHuman { param([string]$RepoPath, [string]$RelativePath) Invoke-GitAi -Label "legacy human checkpoint for $RelativePath" -ArgumentList @("checkpoint", "human", $RelativePath) -WorkingDirectory $RepoPath | Out-Null }

function Get-BlameOutput { param([string]$RepoPath, [string]$RelativePath) return (Invoke-GitAi -Label "blame $RelativePath" -ArgumentList @("blame", $RelativePath) -WorkingDirectory $RepoPath).Output }

function Get-NoteOutput {
    param([string]$RepoPath)
    $note = Invoke-Git -Label "show ai note" -ArgumentList @("notes", "--ref=ai", "show", "HEAD") -WorkingDirectory $RepoPath -AllowFailure
    if ($note.ExitCode -ne 0) { return @("<no ai note>") }
    return $note.Output
}

function Assert-Contains {
    param([string]$Scenario, [string[]]$Lines, [string]$Needle, [string]$FailureMessage)
    if (-not ($Lines | Where-Object { $_.Contains($Needle) })) {
        throw "${Scenario}: $FailureMessage`nExpected to find: $Needle`nOutput:`n$($Lines -join "`n")"
    }
}

function Get-BlameLine {
    param([string]$Scenario, [string[]]$BlameLines, [string]$ExpectedText)
    $matches = @($BlameLines | Where-Object { $_.Contains($ExpectedText) })
    if ($matches.Count -eq 0) {
        throw "${Scenario}: expected line text was missing from blame output: $ExpectedText`nBlame:`n$($BlameLines -join "`n")"
    }
    return $matches[0]
}

function Assert-LineAttributedToAi {
    param([string]$Scenario, [string[]]$BlameLines, [string]$ExpectedText)
    $line = Get-BlameLine -Scenario $Scenario -BlameLines $BlameLines -ExpectedText $ExpectedText
    if (-not $line.Contains("mock_ai")) { throw "${Scenario}: expected line to be attributed to mock_ai: $ExpectedText`nActual blame line:`n$line" }
}

function Assert-LineNotAttributedToAi {
    param([string]$Scenario, [string[]]$BlameLines, [string]$ExpectedText)
    $line = Get-BlameLine -Scenario $Scenario -BlameLines $BlameLines -ExpectedText $ExpectedText
    if ($line.Contains("mock_ai")) { throw "${Scenario}: expected line not to be attributed to mock_ai: $ExpectedText`nActual blame line:`n$line" }
}

function Assert-LineAbsentFromBlame {
    param([string]$Scenario, [string[]]$BlameLines, [string]$UnexpectedText)
    if ($BlameLines | Where-Object { $_.Contains($UnexpectedText) }) {
        throw "${Scenario}: unexpected deleted line remained in blame output: $UnexpectedText`nBlame:`n$($BlameLines -join "`n")"
    }
}

function Show-AttributionState {
    param([string]$Scenario, [string]$RepoPath, [string]$RelativePath)

    $blame = Get-BlameOutput -RepoPath $RepoPath -RelativePath $RelativePath
    $note = Get-NoteOutput -RepoPath $RepoPath

    if ($VerboseOutput) {
        Write-Host "===== $Scenario ====="
        $head = (Invoke-Git -Label "rev-parse HEAD" -ArgumentList @("rev-parse", "HEAD") -WorkingDirectory $RepoPath).Output
        Write-Host "HEAD: $($head[0])"
        Write-Host "--- $RelativePath"
        Get-Content -LiteralPath (Join-Path $RepoPath $RelativePath) | ForEach-Object { Write-Host $_ }
        Write-Host "--- git-ai blame $RelativePath"
        $blame | ForEach-Object { Write-Host $_ }
        Write-Host "--- git notes --ref=ai show HEAD"
        $note | ForEach-Object { Write-Host $_ }
    }

    return [pscustomobject]@{ Blame = [string[]]$blame; Note = [string[]]$note }
}

function Assert-NoteMentionsFile { param([string]$Scenario, [string[]]$NoteLines, [string]$RelativePath) Assert-Contains -Scenario $Scenario -Lines $NoteLines -Needle $RelativePath -FailureMessage "expected AI note to mention file with AI attribution" }

function Initialize-TestRepo {
    param([int]$Iteration)

    $createdTemp = [string]::IsNullOrWhiteSpace($WorkDir)
    if ($createdTemp) {
        $repoRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("git-ai-attribution-scenarios-$Iteration-" + [System.Guid]::NewGuid().ToString("N"))
    } elseif ($Iterations -eq 1) {
        $repoRoot = $WorkDir
    } else {
        $repoRoot = Join-Path $WorkDir ("iteration-$Iteration")
    }

    New-Item -ItemType Directory -Path $repoRoot -Force | Out-Null
    $repoPath = (Resolve-Path -LiteralPath $repoRoot).Path
    $testHome = Join-Path $repoPath ".git-ai-test-home"
    $testConfigDir = Join-Path $testHome ".git-ai"
    $testDbPath = Join-Path (Split-Path -Parent $repoPath) ("git-ai-test-$Iteration-" + [System.Guid]::NewGuid().ToString("N") + ".sqlite")
    $realGitJson = $script:RealGitPath.Replace("\", "\\")

    New-Item -ItemType Directory -Path $testConfigDir -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $testHome ".config") -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $testHome "AppData\Roaming") -Force | Out-Null
    New-Item -ItemType Directory -Path (Join-Path $testHome "AppData\Local") -Force | Out-Null
    [System.IO.File]::WriteAllText((Join-Path $testHome ".gitconfig"), "[user]`n`tname = Test User`n`temail = test@example.com`n", [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::WriteAllText((Join-Path $testConfigDir "config.json"), "{`"git_path`":`"$realGitJson`",`"feature_flags`":{`"async_mode`":false}}", [System.Text.UTF8Encoding]::new($false))

    $script:CommonEnv = @{
        HOME = $testHome
        USERPROFILE = $testHome
        APPDATA = Join-Path $testHome "AppData\Roaming"
        LOCALAPPDATA = Join-Path $testHome "AppData\Local"
        XDG_CONFIG_HOME = Join-Path $testHome ".config"
        GIT_CONFIG_GLOBAL = Join-Path $testHome ".gitconfig"
        GIT_CONFIG_NOSYSTEM = "1"
        GIT_AI_DAEMON_CHECKPOINT_DELEGATE = "false"
        GIT_AI_TEST_DB_PATH = $testDbPath
        GITAI_TEST_DB_PATH = $testDbPath
    }

    Invoke-Git -Label "git init" -ArgumentList @("init") -WorkingDirectory $repoPath | Out-Null
    Invoke-Git -Label "configure user name" -ArgumentList @("config", "user.name", "Test User") -WorkingDirectory $repoPath | Out-Null
    Invoke-Git -Label "configure user email" -ArgumentList @("config", "user.email", "test@example.com") -WorkingDirectory $repoPath | Out-Null

    return [pscustomobject]@{ RepoPath = $repoPath; CreatedTemp = $createdTemp }
}

function Run-ScenarioSuite {
    param([string]$RepoPath)

    $scenario = "01_ai_creates_new_file"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "generated.txt" -Content "AI line 1`nAI line 2`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "generated.txt"
    Commit-All -RepoPath $RepoPath -Message $scenario
    $state = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "generated.txt"
    Assert-NoteMentionsFile -Scenario $scenario -NoteLines $state.Note -RelativePath "generated.txt"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "AI line 1"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "AI line 2"

    $scenario = "02_human_creates_new_file_ai_appends_before_commit"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "mixed-before-first-commit.txt" -Content "Human seed 1`nHuman seed 2`n"
    Checkpoint-KnownHuman -RepoPath $RepoPath -RelativePath "mixed-before-first-commit.txt"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "mixed-before-first-commit.txt" -Content "Human seed 1`nHuman seed 2`nAI append 1`nAI append 2`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "mixed-before-first-commit.txt"
    Commit-All -RepoPath $RepoPath -Message $scenario
    $state = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "mixed-before-first-commit.txt"
    Assert-NoteMentionsFile -Scenario $scenario -NoteLines $state.Note -RelativePath "mixed-before-first-commit.txt"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Human seed 1"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Human seed 2"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "AI append 1"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "AI append 2"

    $scenario = "03_ai_creates_new_file_known_human_appends_before_commit"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "ai-then-known-human.txt" -Content "AI seed 1`nAI seed 2`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "ai-then-known-human.txt"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "ai-then-known-human.txt" -Content "AI seed 1`nAI seed 2`nKnown human append 1`nKnown human append 2`n"
    Checkpoint-KnownHuman -RepoPath $RepoPath -RelativePath "ai-then-known-human.txt"
    Commit-All -RepoPath $RepoPath -Message $scenario
    $state = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "ai-then-known-human.txt"
    Assert-NoteMentionsFile -Scenario $scenario -NoteLines $state.Note -RelativePath "ai-then-known-human.txt"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "AI seed 1"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "AI seed 2"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Known human append 1"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Known human append 2"

    $scenario = "04_ai_creates_new_file_legacy_human_then_ai"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "legacy-hole.txt" -Content "AI before legacy`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "legacy-hole.txt"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "legacy-hole.txt" -Content "AI before legacy`nLegacy human middle`n"
    Checkpoint-LegacyHuman -RepoPath $RepoPath -RelativePath "legacy-hole.txt"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "legacy-hole.txt" -Content "AI before legacy`nLegacy human middle`nAI after legacy`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "legacy-hole.txt"
    Commit-All -RepoPath $RepoPath -Message $scenario
    $state = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "legacy-hole.txt"
    Assert-NoteMentionsFile -Scenario $scenario -NoteLines $state.Note -RelativePath "legacy-hole.txt"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "AI before legacy"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Legacy human middle"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "AI after legacy"

    $scenario = "05_known_human_inserts_between_ai_lines_before_first_commit"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "insert-between-ai.txt" -Content "AI top`nAI bottom`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "insert-between-ai.txt"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "insert-between-ai.txt" -Content "AI top`nKnown human inserted`nAI bottom`n"
    Checkpoint-KnownHuman -RepoPath $RepoPath -RelativePath "insert-between-ai.txt"
    Commit-All -RepoPath $RepoPath -Message $scenario
    $state = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "insert-between-ai.txt"
    Assert-NoteMentionsFile -Scenario $scenario -NoteLines $state.Note -RelativePath "insert-between-ai.txt"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "AI top"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Known human inserted"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "AI bottom"

    $scenario = "06_ai_modifies_existing_human_file"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "existing-human.txt" -Content "human top`nhuman middle`nhuman bottom`n"
    Commit-All -RepoPath $RepoPath -Message "06a_seed_existing_human_file"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "existing-human.txt" -Content "human top`nai changed middle`nhuman bottom`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "existing-human.txt"
    Commit-All -RepoPath $RepoPath -Message $scenario
    $state = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "existing-human.txt"
    Assert-NoteMentionsFile -Scenario $scenario -NoteLines $state.Note -RelativePath "existing-human.txt"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "human top"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "ai changed middle"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "human bottom"

    $scenario = "07_human_modifies_committed_ai_line"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "human-overrides-ai.txt" -Content "ai before`nai to override`nai after`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "human-overrides-ai.txt"
    Commit-All -RepoPath $RepoPath -Message "07a_seed_ai_file"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "human-overrides-ai.txt" -Content "ai before`nhuman override`nai after`n"
    Commit-All -RepoPath $RepoPath -Message $scenario
    $state = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "human-overrides-ai.txt"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "ai before"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "human override"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "ai after"

    $scenario = "08_human_deletes_ai_line"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "human-deletes-ai.txt" -Content "ai keep top`nai delete middle`nai keep bottom`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "human-deletes-ai.txt"
    Commit-All -RepoPath $RepoPath -Message "08a_seed_ai_lines"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "human-deletes-ai.txt" -Content "ai keep top`nai keep bottom`n"
    Commit-All -RepoPath $RepoPath -Message $scenario
    $state = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "human-deletes-ai.txt"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "ai keep top"
    Assert-LineAbsentFromBlame -Scenario $scenario -BlameLines $state.Blame -UnexpectedText "ai delete middle"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "ai keep bottom"

    $scenario = "09_ai_reflows_one_human_line"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "reflow.txt" -Content "call(foo, bar, baz)`n"
    Commit-All -RepoPath $RepoPath -Message "09a_seed_human_reflow"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "reflow.txt" -Content "call(`n  foo,`n  bar,`n  baz`n)`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "reflow.txt"
    Commit-All -RepoPath $RepoPath -Message $scenario
    $state = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "reflow.txt"
    Assert-NoteMentionsFile -Scenario $scenario -NoteLines $state.Note -RelativePath "reflow.txt"
    foreach ($text in @("call(", "  foo,", "  bar,", "  baz", ")")) { Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText $text }

    $scenario = "10_multi_file_scoped_checkpoint"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "scoped-ai.txt" -Content "scoped base`n"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "unscoped-human.txt" -Content "unscoped base`n"
    Commit-All -RepoPath $RepoPath -Message "10a_seed_scoped_files"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "scoped-ai.txt" -Content "scoped base`nscoped ai line`n"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "unscoped-human.txt" -Content "unscoped base`nunscoped human line`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "scoped-ai.txt"
    Commit-All -RepoPath $RepoPath -Message $scenario
    $state = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "scoped-ai.txt"
    Assert-NoteMentionsFile -Scenario $scenario -NoteLines $state.Note -RelativePath "scoped-ai.txt"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "scoped ai line"
    $unscoped = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "unscoped-human.txt"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $unscoped.Blame -ExpectedText "unscoped human line"

    $scenario = "11_before_first_commit_human_copies_ai_lines"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "precommit-human-copy-ai.txt" -Content "AI source 1`nAI source 2`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "precommit-human-copy-ai.txt"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "precommit-human-copy-ai.txt" -Content "AI source 1`nAI source 2`nHuman copied AI source 1`nHuman copied AI source 2`n"
    Checkpoint-KnownHuman -RepoPath $RepoPath -RelativePath "precommit-human-copy-ai.txt"
    Commit-All -RepoPath $RepoPath -Message $scenario
    $state = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "precommit-human-copy-ai.txt"
    Assert-NoteMentionsFile -Scenario $scenario -NoteLines $state.Note -RelativePath "precommit-human-copy-ai.txt"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "AI source 1"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "AI source 2"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Human copied AI source 1"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Human copied AI source 2"

    $scenario = "12_committed_ai_human_copies_ai_lines"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "committed-human-copy-ai.txt" -Content "Committed AI source 1`nCommitted AI source 2`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "committed-human-copy-ai.txt"
    Commit-All -RepoPath $RepoPath -Message "12a_seed_committed_ai_lines"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "committed-human-copy-ai.txt" -Content "Committed AI source 1`nCommitted AI source 2`nHuman copied committed AI source 1`nHuman copied committed AI source 2`n"
    Checkpoint-KnownHuman -RepoPath $RepoPath -RelativePath "committed-human-copy-ai.txt"
    Commit-All -RepoPath $RepoPath -Message $scenario
    $state = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "committed-human-copy-ai.txt"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Committed AI source 1"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Committed AI source 2"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Human copied committed AI source 1"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Human copied committed AI source 2"

    $scenario = "13_before_first_commit_human_adds_and_modifies_ai_code"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "precommit-human-edits-ai.txt" -Content "AI function start`nAI value = 1`nAI function end`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "precommit-human-edits-ai.txt"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "precommit-human-edits-ai.txt" -Content "AI function start`nHuman changed value = 2`nHuman added guard`nAI function end`n"
    Checkpoint-KnownHuman -RepoPath $RepoPath -RelativePath "precommit-human-edits-ai.txt"
    Commit-All -RepoPath $RepoPath -Message $scenario
    $state = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "precommit-human-edits-ai.txt"
    Assert-NoteMentionsFile -Scenario $scenario -NoteLines $state.Note -RelativePath "precommit-human-edits-ai.txt"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "AI function start"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Human changed value = 2"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Human added guard"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "AI function end"

    $scenario = "14_committed_ai_human_modifies_ai_code"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "committed-human-edits-ai.txt" -Content "Committed AI top`nCommitted AI value = 1`nCommitted AI bottom`n"
    Checkpoint-Ai -RepoPath $RepoPath -RelativePath "committed-human-edits-ai.txt"
    Commit-All -RepoPath $RepoPath -Message "14a_seed_committed_ai_code"
    Set-FileContentUtf8 -RepoPath $RepoPath -RelativePath "committed-human-edits-ai.txt" -Content "Committed AI top`nHuman changed committed value = 2`nCommitted AI bottom`n"
    Checkpoint-KnownHuman -RepoPath $RepoPath -RelativePath "committed-human-edits-ai.txt"
    Commit-All -RepoPath $RepoPath -Message $scenario
    $state = Show-AttributionState -Scenario $scenario -RepoPath $RepoPath -RelativePath "committed-human-edits-ai.txt"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Committed AI top"
    Assert-LineNotAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Human changed committed value = 2"
    Assert-LineAttributedToAi -Scenario $scenario -BlameLines $state.Blame -ExpectedText "Committed AI bottom"
}

$script:GitAiPath = Join-Path $RepoRoot "target\debug\git-ai.exe"
$script:RealGitPath = Find-RealGitPath -PreferredGitPath $GitPath
if ($SkipBuild) {
    if (-not (Test-Path -LiteralPath $script:GitAiPath)) { throw "git-ai debug binary not found at $script:GitAiPath and -SkipBuild was provided." }
} else {
    Invoke-CheckedCommand -Label "build git-ai test-support binary" -FilePath "cargo" -ArgumentList @("build", "--bin", "git-ai", "--features", "test-support") -WorkingDirectory $RepoRoot | Out-Null
}
if (-not (Test-Path -LiteralPath $script:GitAiPath)) { throw "git-ai debug binary not found after build: $script:GitAiPath" }

for ($iteration = 1; $iteration -le $Iterations; $iteration++) {
    $context = $null
    $failed = $false
    try {
        $context = Initialize-TestRepo -Iteration $iteration
        Run-ScenarioSuite -RepoPath $context.RepoPath
        Write-Host "[$iteration/$Iterations] passed"
    } catch {
        $failed = $true
        if ($context -and (Test-Path -LiteralPath $context.RepoPath)) {
            Write-Host "[$iteration/$Iterations] failed; repo kept at: $($context.RepoPath)"
        }
        throw
    } finally {
        if ($context -and $context.CreatedTemp -and -not $Keep -and -not $failed -and (Test-Path -LiteralPath $context.RepoPath)) {
            Remove-Item -LiteralPath $context.RepoPath -Recurse -Force
        }
    }
}

Write-Host "All manual attribution scenario iterations passed."
if ($Keep) { Write-Host "Repos kept under: $WorkDir" }
