@echo off
REM Launch perch. Pass a channel to open it, or nothing to reopen the
REM last one. Builds first if the release binary is missing or out of date.
REM
REM   run.cmd              reopen the last channel
REM   run.cmd forsen       open a specific channel
REM   run.cmd forsen --volume 30

setlocal
cd /d "%~dp0"

cargo build --release -p perch || exit /b 1
start "" "target\release\perch.exe" %*
