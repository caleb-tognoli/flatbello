#!/bin/bash
# Runs a command; if it fails, repeats its error lines (or else its last lines) as GitHub
# annotations, which, unlike job logs, anyone can read through the API. Exits with the
# command's status.
#   tools/ci/annotate.sh cargo test --workspace
log=$(mktemp)
set -o pipefail
"$@" 2>&1 | tee "$log"
status=$?
[ "$status" -eq 0 ] && exit 0
lines=$(grep -E "error|Error|LNK[0-9]|cannot find|not found|panicked|FAILED" "$log" | head -n 40)
[ -n "$lines" ] || lines=$(tail -n 20 "$log")
printf '%s\n' "$lines" | while IFS= read -r line; do echo "::error::${line//%/%25}"; done
exit "$status"
