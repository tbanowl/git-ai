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
