#!/bin/bash
# Runs a command; if it fails, repeats its first error lines as GitHub annotations, which (unlike
# job logs) anyone can read through the API.
#   tools/ci/annotate.sh cargo test --workspace
log=$(mktemp)
set -o pipefail
if "$@" 2>&1 | tee "$log"; then
    exit 0
fi
status=$?
grep -E "error|Error|LNK[0-9]|cannot find|not found|panicked|FAILED" "$log" | head -n 40 |
    while IFS= read -r line; do echo "::error::${line//%/%25}"; done
exit "$status"
