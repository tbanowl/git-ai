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
