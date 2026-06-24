$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$defaultApiPath = 'http://10.251.12.24:30939'
$apiPathFromEnv = [Environment]::GetEnvironmentVariable('GIT_AI_API_BASE_URL')
if (-not [string]::IsNullOrWhiteSpace($apiPathFromEnv)) {
    $API_PATH = $apiPathFromEnv.TrimEnd('/')
} else {
    $API_PATH = $defaultApiPath
}
$SENTRY_ENTERPRISE = "$API_PATH/git-ai/dsn" -replace '^(https?://)', '${1}dsn-key@'

$gitAiDaemonHome = [Environment]::GetEnvironmentVariable('GIT_AI_DAEMON_HOME', 'User')
if ([string]::IsNullOrWhiteSpace($gitAiDaemonHome)) {
    try {
        if (Test-Path "Q:\") {
            $daemonHomePath = "Q:\ProgramData\git-ai"

            if (-not (Test-Path $daemonHomePath)) {
                Write-Host "Warning: path does not exist: $daemonHomePath"
            }

            [Environment]::SetEnvironmentVariable('GIT_AI_DAEMON_HOME', $daemonHomePath, 'User')
            $env:GIT_AI_DAEMON_HOME = $daemonHomePath

            Write-Host "Set Environment Variable: GIT_AI_DAEMON_HOME=$daemonHomePath"
        }
    } catch {
        Write-Host "Set Environment Variable 'GIT_AI_DAEMON_HOME' failed: $($_.Exception.Message)"
    }
}

function Write-ErrorAndExit {
    param(
        [Parameter(Mandatory = $true)][string]$Message
    )
    Write-Host "Error: $Message" -ForegroundColor Red
    exit 1
}

function Write-Success {
    param(
        [Parameter(Mandatory = $true)][string]$Message
    )
    Write-Host $Message -ForegroundColor Green
}

function Write-Warning {
    param(
        [Parameter(Mandatory = $true)][string]$Message
    )
    Write-Host $Message -ForegroundColor Yellow
}

function Normalize-PathString {
    param(
        [Parameter(Mandatory = $true)][string]$Path
    )

    try {
        return ([IO.Path]::GetFullPath($Path.Trim())).TrimEnd('\').ToLowerInvariant()
    } catch {
        return ($Path.Trim()).TrimEnd('\').ToLowerInvariant()
    }
}

function Test-FileAvailable {
    param(
        [Parameter(Mandatory = $true)][string]$Path
    )

    try {
        $stream = [System.IO.File]::Open($Path, 'Open', 'Write', 'None')
        $stream.Close()
        return $true
    } catch {
        return $false
    }
}

function Stop-GitAiBackgroundService {
    param(
        [Parameter(Mandatory = $true)][string]$GitAiExe,
        [Parameter(Mandatory = $false)][switch]$Hard
    )

    if (-not (Test-Path -LiteralPath $GitAiExe)) {
        return $false
    }

    $args = @('bg', 'shutdown')
    if ($Hard) {
        $args += '--hard'
    }

    try {
        & $GitAiExe @args *> $null
        return $LASTEXITCODE -eq 0
    } catch {
        return $false
    }
}

function Get-GitAiManagedProcesses {
    param(
        [Parameter(Mandatory = $true)][string]$InstallDir
    )

    $targetPaths = @(
        (Normalize-PathString (Join-Path $InstallDir 'git-ai.exe')),
        (Normalize-PathString (Join-Path $InstallDir 'git.exe'))
    )

    $processes = @(Get-CimInstance Win32_Process -ErrorAction SilentlyContinue | Where-Object {
            $_.ProcessId -ne $PID -and
            $_.ExecutablePath -and
            ($targetPaths -contains (Normalize-PathString $_.ExecutablePath))
        })

    return $processes
}

function Stop-GitAiManagedProcesses {
    param(
        [Parameter(Mandatory = $true)][string]$InstallDir
    )

    $processes = @(Get-GitAiManagedProcesses -InstallDir $InstallDir)
    if ($processes.Count -eq 0) {
        return $false
    }

    $pids = @($processes | Sort-Object ProcessId -Unique | Select-Object -ExpandProperty ProcessId)
    Write-Warning ("Stopping lingering git-ai processes: {0}" -f ($pids -join ', '))

    foreach ($processId in $pids) {
        try {
            Stop-Process -Id $processId -Force -ErrorAction Stop
        } catch { }
    }

    return $true
}

function Wait-ForFileAvailable {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string]$InstallDir,
        [Parameter(Mandatory = $false)][int]$MaxWaitSeconds = 300,
        [Parameter(Mandatory = $false)][int]$RetryIntervalSeconds = 5,
        [Parameter(Mandatory = $false)][int]$ForceKillAfterSeconds = 20
    )
    
    $elapsed = 0
    $gitAiExe = Join-Path $InstallDir 'git-ai.exe'

    [void](Stop-GitAiBackgroundService -GitAiExe $gitAiExe)

    while ($elapsed -lt $MaxWaitSeconds) {
        if (Test-FileAvailable -Path $Path) {
            return $true
        }

        if ($elapsed -ge $ForceKillAfterSeconds) {
            [void](Stop-GitAiBackgroundService -GitAiExe $gitAiExe -Hard)
            [void](Stop-GitAiManagedProcesses -InstallDir $InstallDir)
        }

        if (-not (Test-FileAvailable -Path $Path)) {
            if ($elapsed -eq 0) {
                Write-Host "Waiting for file to be available: $Path" -ForegroundColor Yellow
            }
            Start-Sleep -Seconds $RetryIntervalSeconds
            $elapsed += $RetryIntervalSeconds
        }
    }
    return $false
}

function Verify-Checksum {
    param(
        [Parameter(Mandatory = $true)][string]$File,
        [Parameter(Mandatory = $true)][string]$BinaryName
    )

    # Skip verification if no checksums are embedded
    if ($EmbeddedChecksums -eq '__CHECKSUMS_PLACEHOLDER__') {
        return
    }

    # Extract expected checksum for this binary
    $expected = $null
    $entries = $EmbeddedChecksums -split '\|'
    foreach ($entry in $entries) {
        if ($entry -match "^([0-9a-fA-F]+)\s+$([regex]::Escape($BinaryName))$") {
            $expected = $Matches[1]
            break
        }
    }

    if (-not $expected) {
        Write-ErrorAndExit "No checksum found for $BinaryName"
    }

    # Calculate actual checksum
    $hashCommand = Get-Command Get-FileHash -ErrorAction SilentlyContinue
    if ($null -ne $hashCommand) {
        $actual = (Get-FileHash -Path $File -Algorithm SHA256).Hash.ToLower()
    } else {
        $stream = [System.IO.File]::OpenRead($File)
        try {
            $sha256 = [System.Security.Cryptography.SHA256]::Create()
            $hashBytes = $sha256.ComputeHash($stream)
            $actual = ([System.BitConverter]::ToString($hashBytes)).Replace('-', '').ToLower()
        } finally {
            $stream.Dispose()
            if ($sha256) {
                $sha256.Dispose()
            }
        }
    }

    if ($expected -ne $actual) {
        Remove-Item -Force -ErrorAction SilentlyContinue $File
        Write-ErrorAndExit "Checksum verification failed for $BinaryName`nExpected: $expected`nActual:   $actual"
    }

    Write-Success "Checksum verified for $BinaryName"
}

function Resolve-ServiceExecutablePath {
    param(
        [Parameter(Mandatory = $true)][string]$PathName
    )

    $trimmed = $PathName.Trim()
    if ($trimmed -match '^\"([^\"]+\.exe)\"') {
        return $Matches[1]
    }

    $exeIndex = $trimmed.IndexOf('.exe', [StringComparison]::OrdinalIgnoreCase)
    if ($exeIndex -ge 0) {
        return $trimmed.Substring(0, $exeIndex + 4).Trim()
    }

    return $trimmed
}

function Read-TextFilePreservingEncoding {
    param(
        [Parameter(Mandatory = $true)][string]$Path
    )

    $reader = New-Object System.IO.StreamReader($Path, [System.Text.Encoding]::Default, $true)
    try {
        $content = $reader.ReadToEnd()
        $encoding = $reader.CurrentEncoding
    } finally {
        $reader.Dispose()
    }

    $lines = New-Object System.Collections.Generic.List[string]
    $normalizedContent = $content -replace "`r`n", "`n" -replace "`r", "`n"
    if ($normalizedContent.Length -gt 0) {
        $lines.AddRange([string[]]($normalizedContent -split "`n"))
        if ($normalizedContent.EndsWith("`n")) {
            $lines.RemoveAt($lines.Count - 1)
        }
    }

    return [PSCustomObject]@{
        Lines    = $lines
        Encoding = $encoding
    }
}

function Get-IniSectionRange {
    param(
        [Parameter(Mandatory = $true)][System.Collections.Generic.List[string]]$Lines,
        [Parameter(Mandatory = $true)][string]$Section
    )

    $start = -1
    $sectionPattern = '^\s*\[' + [regex]::Escape($Section) + '\]\s*$'
    for ($i = 0; $i -lt $Lines.Count; $i++) {
        if ($Lines[$i] -match $sectionPattern) {
            $start = $i
            break
        }
    }

    if ($start -eq -1) {
        return [PSCustomObject]@{
            Exists = $false
            Start  = -1
            End    = -1
        }
    }

    $end = $Lines.Count - 1
    for ($j = $start + 1; $j -lt $Lines.Count; $j++) {
        if ($Lines[$j] -match '^\s*\[[^\]]+\]\s*$') {
            $end = $j - 1
            break
        }
    }

    return [PSCustomObject]@{
        Exists = $true
        Start  = $start
        End    = $end
    }
}

function Ensure-IniSection {
    param(
        [Parameter(Mandatory = $true)][System.Collections.Generic.List[string]]$Lines,
        [Parameter(Mandatory = $true)][string]$Section
    )

    $range = Get-IniSectionRange -Lines $Lines -Section $Section
    if (-not $range.Exists) {
        if ($Lines.Count -gt 0 -and $Lines[$Lines.Count - 1].Trim() -ne '') {
            [void]$Lines.Add('')
        }
        [void]$Lines.Add("[$Section]")
    }
}

function Set-IniValue {
    param(
        [Parameter(Mandatory = $true)][System.Collections.Generic.List[string]]$Lines,
        [Parameter(Mandatory = $true)][string]$Section,
        [Parameter(Mandatory = $true)][string]$Key,
        [Parameter(Mandatory = $true)][string]$Value
    )

    Ensure-IniSection -Lines $Lines -Section $Section
    $range = Get-IniSectionRange -Lines $Lines -Section $Section
    $keyPattern = '^\s*' + [regex]::Escape($Key) + '\s*='

    for ($i = $range.Start + 1; $i -le $range.End; $i++) {
        if ($Lines[$i] -match $keyPattern) {
            if ($Lines[$i] -ne "$Key=$Value") {
                $Lines[$i] = "$Key=$Value"
                return $true
            }
            return $false
        }
    }

    $Lines.Insert($range.End + 1, "$Key=$Value")
    return $true
}

function Normalize-IniValue {
    param(
        [Parameter(Mandatory = $true)][string]$Value
    )

    return $Value.Trim().Trim('"').Trim("'").TrimEnd('\').ToLowerInvariant()
}

# GitHub repository details
# Replaced during release builds with the actual repository (e.g., "git-ai-project/git-ai")
# When set to __REPO_PLACEHOLDER__, defaults to "git-ai-project/git-ai"
$Repo = '__REPO_PLACEHOLDER__'
if ($Repo -eq '__REPO_PLACEHOLDER__') {
    $Repo = 'git-ai-project/git-ai'
}

# Version placeholder - replaced during release builds with actual version (e.g., "v1.0.24")
# When set to __VERSION_PLACEHOLDER__, defaults to "latest"
$PinnedVersion = '__VERSION_PLACEHOLDER__'

# Embedded checksums - replaced during release builds with actual SHA256 checksums
# Format: "hash  filename|hash  filename|..." (pipe-separated)
# When set to __CHECKSUMS_PLACEHOLDER__, checksum verification is skipped
$EmbeddedChecksums = '__CHECKSUMS_PLACEHOLDER__'

# Ensure TLS 1.2 for GitHub downloads on older PowerShell versions
try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
} catch { }

function Get-Architecture {
    try {
        $arch = [System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture
        switch ($arch) {
            'X64' { return 'x64' }
            'Arm64' { return 'arm64' }
            default { return $null }
        }
    } catch {
        $pa = $env:PROCESSOR_ARCHITECTURE
        if ($pa -match 'ARM64') { return 'arm64' }
        elseif ($pa -match '64') { return 'x64' }
        else { return $null }
    }
}

function Get-StdGitPath {
    $cmd = Get-Command git.exe -ErrorAction SilentlyContinue
    $gitPath = $null
    if ($cmd -and $cmd.Path) {
        # Ensure we never return a path for git that contains git-ai (recursive)
        if ($cmd.Path -notmatch "git-ai") {
            $gitPath = $cmd.Path
        }
    }

    # If detection failed or was our own shim, try to recover from saved config
    if (-not $gitPath) {
        try {
            $cfgPath = Join-Path $HOME ".git-ai\config.json"
            if (Test-Path -LiteralPath $cfgPath) {
                $cfg = Get-Content -LiteralPath $cfgPath -Raw | ConvertFrom-Json
                if ($cfg -and $cfg.git_path -and ($cfg.git_path -notmatch 'git-ai') -and (Test-Path -LiteralPath $cfg.git_path)) {
                    $gitPath = $cfg.git_path
                }
            }
        } catch { }
    }

    if (-not $gitPath) {
        try {
            $gitPath = Convert-Path (git-ai git-path)
        } catch {

        }
    }

    # If still not found, fail with a clear message
    if (-not $gitPath) {
        Write-ErrorAndExit "Could not detect a standard git binary on PATH. Please ensure you have Git installed and available on your PATH. If you believe this is a bug with the installer, please file an issue at https://github.com/git-ai-project/git-ai/issues."
    }

    try {
        & $gitPath --version | Out-Null
        if ($LASTEXITCODE -ne 0) { throw 'bad' }
    } catch {
        Write-ErrorAndExit "Detected git at $gitPath is not usable (--version failed). Please ensure you have Git installed and available on your PATH. If you believe this is a bug with the installer, please file an issue at https://github.com/git-ai-project/git-ai/issues."
    }

    return $gitPath
}

# Update PATH so git-ai (cmd + bin) takes precedence over native Git.
# Windows concatenates Machine PATH before User PATH, so native Git left in the
# Machine PATH would always shadow git-ai. Behavior by caller identity:
#   * Admin + native git in Machine PATH: remove native git from Machine PATH
#     and migrate it into the User PATH right after cmd+bin.
#   * Non-admin + native git in Machine PATH: cannot touch Machine PATH -> emit
#     a warning (git still resolves to native git) but still add git-ai to User PATH.
# git-ai paths are NEVER written to the Machine PATH.
# Target User PATH order: <cmd>; <bin>; <native-git-if-applicable>; ...
function Update-GitAiPath {
    param(
        [Parameter(Mandatory = $true)][string]$CmdDir,
        [Parameter(Mandatory = $true)][string]$BinDir,
        [Parameter(Mandatory = $true)][string]$OrigGitDir
    )

    $sep = ';'

    function NormalizePath([string]$p) {
        try { return ([IO.Path]::GetFullPath($p.Trim())).TrimEnd('\').ToLowerInvariant() }
        catch { return ($p.Trim()).TrimEnd('\').ToLowerInvariant() }
    }

    function Split-PathEntries([string]$PathString) {
        if ($PathString) {
            return @(($PathString -split $sep) | Where-Object { $_ -and $_.Trim() -ne '' })
        }
        return @()
    }

    $isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

    $normCmd = NormalizePath $CmdDir
    $normBin = NormalizePath $BinDir
    $normOrig = NormalizePath $OrigGitDir

    $machineEntries = Split-PathEntries ([Environment]::GetEnvironmentVariable('Path', 'Machine'))
    $userEntries = Split-PathEntries ([Environment]::GetEnvironmentVariable('Path', 'User'))

    $origGitInMachine = $false
    foreach ($e in $machineEntries) { if ((NormalizePath $e) -eq $normOrig) { $origGitInMachine = $true; break } }
    $origGitInUser = $false
    foreach ($e in $userEntries) { if ((NormalizePath $e) -eq $normOrig) { $origGitInUser = $true; break } }

    $machineStatus = 'NoChange'
    # Native git ends up in the User PATH block if it was migrated (admin) or was already there.
    $placeOrigInUser = $origGitInUser
    $warning = $null

    if ($isAdmin -and $origGitInMachine) {
        # Remove native git directory from the system PATH.
        $newMachine = ($machineEntries | Where-Object { (NormalizePath $_) -ne $normOrig }) -join $sep
        try {
            [Environment]::SetEnvironmentVariable('Path', $newMachine, 'Machine')
            $machineStatus = 'RemovedOrigGit'
            $placeOrigInUser = $true
        } catch {
            $machineStatus = 'Error'
        }
    }

    if (-not $isAdmin -and $origGitInMachine) {
        $warning = "Native Git is in the system PATH, which takes precedence over the user PATH. The 'git' command will keep resolving to native Git until you re-run this installer as Administrator. (git-ai itself was added to the user PATH.)"
    }

    # Build the git-ai block: cmd; bin; [native git]
    $block = @($CmdDir, $BinDir)
    if ($placeOrigInUser) { $block += $OrigGitDir }

    # Rebuild User PATH: block first, then remaining unique entries (excluding cmd/bin/orig).
    $reserved = New-Object 'System.Collections.Generic.HashSet[string]'
    [void]$reserved.Add($normCmd)
    [void]$reserved.Add($normBin)
    if ($placeOrigInUser) { [void]$reserved.Add($normOrig) }

    $rebuilt = New-Object System.Collections.Generic.List[string]
    foreach ($b in $block) { [void]$rebuilt.Add($b) }
    foreach ($e in $userEntries) {
        $n = NormalizePath $e
        if (-not $reserved.Contains($n)) { [void]$reserved.Add($n); [void]$rebuilt.Add($e) }
    }
    $newUserPath = $rebuilt -join $sep

    $userStatus = 'NoChange'
    if ($newUserPath -ne ($userEntries -join $sep)) {
        try {
            [Environment]::SetEnvironmentVariable('Path', $newUserPath, 'User')
            $userStatus = 'Updated'
        } catch {
            $userStatus = 'Error'
        }
    } else {
        $userStatus = 'AlreadyPresent'
    }

    # Mirror the precedence into the current process PATH for this session.
    try {
        $procEntries = Split-PathEntries $env:PATH
        $procReserved = New-Object 'System.Collections.Generic.HashSet[string]'
        [void]$procReserved.Add($normCmd)
        [void]$procReserved.Add($normBin)
        if ($placeOrigInUser) { [void]$procReserved.Add($normOrig) }
        $procList = New-Object System.Collections.Generic.List[string]
        foreach ($b in $block) { [void]$procList.Add($b) }
        foreach ($e in $procEntries) {
            $n = NormalizePath $e
            if (-not $procReserved.Contains($n)) { [void]$procReserved.Add($n); [void]$procList.Add($e) }
        }
        $env:PATH = $procList -join $sep
    } catch { }

    if ($warning) {
        Write-Host ''
        Write-Host ('WARNING: ' + $warning) -ForegroundColor Red
        Write-Host 'Re-run this installer as Administrator so native Git can be migrated and git-ai takes precedence.' -ForegroundColor Red
        Write-Host ''
    }

    return [PSCustomObject]@{
        IsAdmin          = $isAdmin
        OrigGitInMachine = $origGitInMachine
        OrigGitInUser    = $origGitInUser
        MachineStatus    = $machineStatus
        UserStatus       = $userStatus
        Warning          = $warning
    }
}

# Detect standard Git early and validate (fail-fast behavior)
$stdGitPath = Get-StdGitPath

# Detect architecture and OS
$arch = Get-Architecture
if (-not $arch) { Write-ErrorAndExit "Unsupported architecture: $([System.Runtime.InteropServices.RuntimeInformation]::OSArchitecture)" }
$os = 'windows'

# Determine binary name and download URLs
# $binaryName = "git-ai"
$binaryName = "git-ai-$os-$arch"

# Determine release tag
# Priority: 1. Local binary override, 2. Pinned version (for release builds), 3. Environment variable, 4. "latest"
if (-not [string]::IsNullOrWhiteSpace($env:GIT_AI_LOCAL_BINARY)) {
    $releaseTag = 'local'
    $abstractBinary = Join-Path (Get-Location) "$binaryName.exe"
} elseif ($PinnedVersion -ne '__VERSION_PLACEHOLDER__') {
    # Version-pinned install script from a release
    $releaseTag = $PinnedVersion
    $downloadUrlExe = "$API_PATH/releases/download/$releaseTag/$binaryName.exe"
    $downloadUrlNoExt = "$API_PATH/releases/download/$releaseTag/$binaryName"
} elseif (-not [string]::IsNullOrWhiteSpace($env:GIT_AI_RELEASE_TAG) -and $env:GIT_AI_RELEASE_TAG -ne 'latest') {
    # Environment variable override
    $releaseTag = $env:GIT_AI_RELEASE_TAG
    $downloadUrlExe = "$API_PATH/releases/download/$releaseTag/$binaryName.exe"
    $downloadUrlNoExt = "$API_PATH/releases/download/$releaseTag/$binaryName"
} else {
    # Default to latest
    $releaseTag = 'latest'
    $downloadUrlExe = "$API_PATH/releases/latest/download/$binaryName.exe"
    $downloadUrlNoExt = "$API_PATH/releases/latest/download/$binaryName"
}

# Install directory: %USERPROFILE%\.git-ai\bin
$installDir = Join-Path $HOME ".git-ai\bin"
New-Item -ItemType Directory -Force -Path $installDir | Out-Null

Write-Host ("Downloading git-ai (release: {0})..." -f $releaseTag)
$tmpFile = Join-Path $installDir "git-ai.tmp.$PID.exe"

function Try-Download {
    param(
        [Parameter(Mandatory = $true)][string]$Url
    )
     try {
        # Disable progress bar to avoid extreme slowdown caused by PowerShell's
        # progress-stream rendering (can make downloads 10-50x slower).
        $oldProgressPreference = $ProgressPreference
        $ProgressPreference = 'SilentlyContinue'
        try {
            Invoke-WebRequest -Uri $Url -OutFile $tmpFile -UseBasicParsing -ErrorAction Stop
        } finally {
            $ProgressPreference = $oldProgressPreference
        }
        return $true
    } catch {
        return $false
    }
}

# Track which download URL succeeded for checksum verification
$downloadedBinaryName = $null
if (Test-Path -LiteralPath $abstractBinary) {
    Copy-Item -Force -Path $abstractBinary -Destination $tmpFile
    $downloadedBinaryName = "$binaryName.exe"
} elseif (-not [string]::IsNullOrWhiteSpace($env:GIT_AI_LOCAL_BINARY)) {
    if (-not (Test-Path -LiteralPath $env:GIT_AI_LOCAL_BINARY)) {
        Remove-Item -Force -ErrorAction SilentlyContinue $tmpFile
        Write-ErrorAndExit "Local binary not found at $($env:GIT_AI_LOCAL_BINARY)"
    }
    Copy-Item -Force -Path $env:GIT_AI_LOCAL_BINARY -Destination $tmpFile
    $downloadedBinaryName = "$binaryName.exe"
} elseif (Try-Download -Url $downloadUrlExe) {
    $downloadedBinaryName = "$binaryName.exe"
} elseif (Try-Download -Url $downloadUrlNoExt) {
    $downloadedBinaryName = $binaryName
}

if (-not $downloadedBinaryName) {
    Remove-Item -Force -ErrorAction SilentlyContinue $tmpFile
    Write-ErrorAndExit 'Failed to download binary (HTTP error)'
}

try {
    if ((Get-Item $tmpFile).Length -le 0) {
        Remove-Item -Force -ErrorAction SilentlyContinue $tmpFile
        Write-ErrorAndExit 'Downloaded file is empty'
    }
} catch {
    Remove-Item -Force -ErrorAction SilentlyContinue $tmpFile
    Write-ErrorAndExit 'Download failed'
}

# Verify checksum if embedded (release builds only)
Verify-Checksum -File $tmpFile -BinaryName $downloadedBinaryName

$finalExe = Join-Path $installDir 'git-ai.exe'

# Wait for git-ai.exe to be available if it exists and is in use
if (Test-Path -LiteralPath $finalExe) {
    if (-not (Wait-ForFileAvailable -Path $finalExe -InstallDir $installDir -MaxWaitSeconds 300 -RetryIntervalSeconds 5)) {
        Remove-Item -Force -ErrorAction SilentlyContinue $tmpFile
        Write-ErrorAndExit "Timeout waiting for $finalExe to be available. Please close any running git-ai processes and try again."
    }
}

Move-Item -Force -Path $tmpFile -Destination $finalExe
try { Unblock-File -Path $finalExe -ErrorAction SilentlyContinue } catch { }

# Create a shim so calling `git` goes through git-ai by PATH precedence
$gitShim = Join-Path $installDir 'git.exe'

# Wait for git.exe shim to be available if it exists and is in use
if (Test-Path -LiteralPath $gitShim) {
    if (-not (Wait-ForFileAvailable -Path $gitShim -InstallDir $installDir -MaxWaitSeconds 300 -RetryIntervalSeconds 5)) {
        Write-ErrorAndExit "Timeout waiting for $gitShim to be available. Please close any running git processes and try again."
    }
}

Copy-Item -Force -Path $finalExe -Destination $gitShim
try { Unblock-File -Path $gitShim -ErrorAction SilentlyContinue } catch { }

# Create a shim so calling `git-og` invokes the standard Git
$gitOgShim = Join-Path $installDir 'git-og.cmd'
$gitOgShimContent = "@echo off$([Environment]::NewLine)`"$stdGitPath`" %*$([Environment]::NewLine)"
Set-Content -Path $gitOgShim -Value $gitOgShimContent -Encoding ASCII -Force
try { Unblock-File -Path $gitOgShim -ErrorAction SilentlyContinue } catch { }

# Create the cmd shim directory: git.cmd / git.ps1 forward to bin\git.exe (proxy
# mode via main.rs argv[0] dispatch). PATH ordering is cmd -> bin -> native git,
# so `git` resolves to cmd\git.cmd first.
$cmdDir = Join-Path $HOME ".git-ai\cmd"
New-Item -ItemType Directory -Force -Path $cmdDir | Out-Null

# git.cmd content mirrors scripts/run-git-proxy.cmd (calls ~/.git-ai/bin/git.exe).
$gitCmdShim = @'
@echo off
setlocal

set "GIT_PROXY=%USERPROFILE%\.git-ai\bin\git.exe"

if not exist "%GIT_PROXY%" (
  echo git.exe not found: "%GIT_PROXY%" 1>&2
  exit /b 1
)

if "%~1"=="" (
  "%GIT_PROXY%" --version
) else (
  "%GIT_PROXY%" %*
)

exit /b %ERRORLEVEL%
'@
Set-Content -Path (Join-Path $cmdDir 'git.cmd') -Value $gitCmdShim -Encoding ASCII -Force

# git.ps1 content mirrors scripts/run-git-proxy.ps1 (calls ~/.git-ai/bin/git.exe).
$gitPs1Shim = @'
$ErrorActionPreference = 'Stop'

$gitProxy = Join-Path $env:USERPROFILE '.git-ai\bin\git.exe'

if (-not (Test-Path -LiteralPath $gitProxy)) {
    Write-Error "git.exe not found: $gitProxy"
    exit 1
}

if ($args.Count -eq 0) {
    & $gitProxy --version
} else {
    & $gitProxy @args
}

exit $LASTEXITCODE
'@
Set-Content -Path (Join-Path $cmdDir 'git.ps1') -Value $gitPs1Shim -Encoding ASCII -Force  # ASCII: content is ASCII-only; avoids PS 5.1's UTF-8 BOM

# Update PATH so git-ai (cmd + bin) takes precedence over native Git.
$origGitDir = Split-Path $stdGitPath -Parent
$skipPathUpdate = $env:GIT_AI_SKIP_PATH_UPDATE -eq '1'
if ($skipPathUpdate) {
    Write-Warning 'Skipping PATH updates because GIT_AI_SKIP_PATH_UPDATE=1'
    $pathUpdate = [PSCustomObject]@{
        UserStatus    = 'Skipped'
        MachineStatus = 'Skipped'
    }
} else {
    $pathUpdate = Update-GitAiPath -CmdDir $cmdDir -BinDir $installDir -OrigGitDir $origGitDir
}

switch ($pathUpdate.UserStatus) {
    'Updated'        { Write-Success 'Added git-ai (cmd + bin) to the user PATH.' }
    'AlreadyPresent' { Write-Success 'git-ai (cmd + bin) already present in the user PATH.' }
    'Error'          { Write-Host 'Failed to update the user PATH.' -ForegroundColor Red }
    default { }
}

switch ($pathUpdate.MachineStatus) {
    'RemovedOrigGit' { Write-Success "Migrated native Git from system PATH to user PATH ($origGitDir)." }
    'NoChange'       { Write-Success 'System PATH left unchanged (git-ai is user-scoped).' }
    'Error'          { Write-Host 'Failed to update the system PATH.' -ForegroundColor Red }
    default { }
}

Write-Success "Successfully installed git-ai into $installDir"
Write-Success "You can now run 'git-ai' from your terminal"

# Configure Git Bash shell profiles so git-ai takes precedence over /mingw64/bin/git
# Git Bash (MSYS2/MinGW) prepends its own directories to PATH, which shadows
# the Windows PATH entry we set above. Writing to ~/.bashrc ensures git-ai's
# bin directory is prepended after Git Bash's own PATH setup.
$gitBashConfigured = $false
$gitBashAlreadyConfigured = $false
try {
    $bashrcPath = Join-Path $HOME '.bashrc'
    $bashProfilePath = Join-Path $HOME '.bash_profile'
    $pathCmd = 'export PATH="$HOME/.git-ai/bin:$PATH"'
    $markerString = '.git-ai/bin'

    # Detect if Git Bash is installed
    $gitBashInstalled = $false
    $gitForWindowsPaths = @()
    if ($env:ProgramFiles) { $gitForWindowsPaths += Join-Path $env:ProgramFiles 'Git\bin\bash.exe' }
    if (${env:ProgramFiles(x86)}) { $gitForWindowsPaths += Join-Path ${env:ProgramFiles(x86)} 'Git\bin\bash.exe' }
    if ($env:LOCALAPPDATA) { $gitForWindowsPaths += Join-Path $env:LOCALAPPDATA 'Programs\Git\bin\bash.exe' }
    foreach ($p in $gitForWindowsPaths) {
        if ($p -and (Test-Path -LiteralPath $p)) {
            $gitBashInstalled = $true
            break
        }
    }

    if ($gitBashInstalled) {
        # Determine which config file to update (prefer .bashrc, fall back to .bash_profile)
        $targetBashConfig = $null
        if (Test-Path -LiteralPath $bashrcPath) {
            $targetBashConfig = $bashrcPath
        } elseif (Test-Path -LiteralPath $bashProfilePath) {
            $targetBashConfig = $bashProfilePath
    } else {
            # No existing config; create .bashrc
            $targetBashConfig = $bashrcPath
        }

        # Check if already configured
        $alreadyPresent = $false
        if (Test-Path -LiteralPath $targetBashConfig) {
            $content = Get-Content -LiteralPath $targetBashConfig -Raw -ErrorAction SilentlyContinue
            if ($content -and $content.Contains($markerString)) {
                $alreadyPresent = $true
            }
        }

        if ($alreadyPresent) {
            $gitBashAlreadyConfigured = $true
        } else {
            $timestamp = Get-Date -Format 'yyyy-MM-dd HH:mm:ss'
            $appendContent = "`n# Added by git-ai installer on $timestamp`n$pathCmd`n"
            $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
            [System.IO.File]::AppendAllText($targetBashConfig, $appendContent, $utf8NoBom)
            $gitBashConfigured = $true
        }
    }
} catch {
    Write-Host "Warning: Failed to configure Git Bash: $($_.Exception.Message)" -ForegroundColor Yellow
}

if ($gitBashConfigured) {
    Write-Success "Successfully configured Git Bash ($targetBashConfig)"
} elseif ($gitBashAlreadyConfigured) {
    Write-Success "Git Bash already configured ($targetBashConfig)"
}

# Write JSON config at %USERPROFILE%\.git-ai\config.json (only if it doesn't exist)
try {
    $configDir = Join-Path $HOME '.git-ai'
    $configJsonPath = Join-Path $configDir 'config.json'
    New-Item -ItemType Directory -Force -Path $configDir | Out-Null

    if (-not (Test-Path -LiteralPath $configJsonPath)) {
        $cfg = @{
            git_path = $stdGitPath
            feature_flags = @{
                async_mode = $true
            }
        } | ConvertTo-Json -Depth 3 -Compress
        $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
        [System.IO.File]::WriteAllText($configJsonPath, $cfg, $utf8NoBom)
    }
} catch {
    Write-Host "Warning: Failed to write config.json: $($_.Exception.Message)" -ForegroundColor Yellow
}

# Config Init
try {
    & $finalExe config set telemetry_enterprise_dsn "$SENTRY_ENTERPRISE"
    Write-Success 'Successfully config telemetry_enterprise_dsn.'
} catch {
    Write-Success 'Warning: Failed config telemetry_enterprise_dsn.'
}

# If nonce exchange failed, run interactive login
Write-Host ''
Write-Host 'Launching login...'
# & $finalExe login

Write-Host "Config notes_store to rest"
try {
    & $finalExe config set notes_store "rest"
    & $finalExe config set api_key "git-ai123456789"
    # & $finalExe config set feature_flags.async_mode "false"
    Write-Success 'Successfully config notes_store to rest.'
} catch {
    Write-Success 'Warning: Failed config notes_store to rest.'
}

# Install hooks
Write-Host 'Setting up IDE/agent hooks...'
try {
    & $finalExe uninstall-hooks
    & $finalExe install-hooks
    # & $finalExe install-hooks | Out-Host
    Write-Success 'Successfully set up IDE/agent hooks'
} catch {
    Write-Warning "Warning: Failed to set up IDE/agent hooks. Please try running 'git-ai install-hooks' manually."
}

try {
    & $finalExe bg shutdown --hard
} catch {

}
Write-Host 'Close and reopen your terminal and IDE sessions to use git-ai.' -ForegroundColor Yellow
