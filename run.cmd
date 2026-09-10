@echo off
REM Launch perch. Pass a channel to open it, or nothing to open on the
REM follows page. Builds first if the release binary is missing or out of date.
REM
REM   run.cmd              open on the follows page
REM   run.cmd forsen       open a specific channel
REM   run.cmd forsen --volume 30

setlocal
cd /d "%~dp0"

cargo build --release -p perch || exit /b 1
start "" "target\release\perch.exe" %*
