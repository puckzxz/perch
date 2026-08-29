#!/bin/sh
# Launch perch. Pass a channel to open it, or nothing to reopen the
# last one. Builds first if the release binary is missing or out of date.
#
#   ./run.sh              reopen the last channel
#   ./run.sh forsen       open a specific channel
#   ./run.sh forsen --volume 30
#
# The counterpart to run.cmd. It runs the binary in the foreground rather
# than detaching the way `start` does on Windows: a release build is windowed,
# so there is nothing to read in the terminal anyway, and staying attached
# means Ctrl-C works and a crash is visible without going to find the log.

set -e
cd "$(dirname "$0")"

cargo build --release -p perch
exec ./target/release/perch "$@"
