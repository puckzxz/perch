#!/usr/bin/env bash
# Ask, once each, every service perch rides without a contract, and say which
# still answer. Run by hand before a release; never in CI.
#
# Each of these fails quietly in the app on purpose — a notice row, an empty
# Recommended group, emotes left as words — so a retired query or a renamed
# rendition can go unnoticed for weeks. Twitch retired the query streamlink
# used for access tokens in November 2025; the SideNav hash was last seen
# answering by its own comment's date (`twitch_api::recommend`).
#
#   twitch-api   SideNav, last broadcasts and channel names (anonymous GQL)
#   twitch-chat  the chat replay query, and the recent-messages service
#   emotes       FrankerFaceZ, BetterTTV and 7TV global sets, and the
#                overlays and effects each one marks
#   streamlink   every rendition the installed streamlink names, read by
#                streamlink::quality, on a live channel
#
# Kept out of CI because anonymous GQL from a shared runner's address invites
# blocks and flaky failures. Sends no token anywhere. PERCH_LIVE_CHANNEL and
# PERCH_LIVE_VIDEO pick what is asked about; see each tests/live.rs.

set -u
cd "$(dirname "$0")/.."

failed=()
for crate in twitch-api twitch-chat emotes streamlink; do
    echo "== $crate"
    if ! cargo test --locked -p "$crate" --test live -- --ignored --nocapture --test-threads=1; then
        failed+=("$crate")
    fi
done

echo
if [ ${#failed[@]} -eq 0 ]; then
    echo "live-check: every service answered"
else
    echo "live-check: FAILED: ${failed[*]}"
    exit 1
fi
