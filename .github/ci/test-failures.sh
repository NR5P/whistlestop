#!/usr/bin/env bash
# Turns a failed `cargo test` run's output into GitHub annotations (seen on
# the run's page without signing in) and a step summary, so failures can be
# read by anyone:  .github/ci/test-failures.sh test-output.txt
# Each failed test, each panic (thread name, message, where), and each
# test binary that didn't finish (a crash, a signal).
set -u
out="$1"
os="${RUNNER_OS:-local}"
esc() { sed -e 's/%/%25/g' -e 's/\r/%0D/g'; }

# Failed tests, by name.
grep -E '^test .* \.\.\. FAILED$' "$out" | sed -E 's/^test (.*) \.\.\. FAILED$/\1/' | sort -u | head -20 |
  while IFS= read -r t; do echo "::error title=$os: FAILED::$t" | esc; done

# Panics: "thread 'NAME' (id) panicked at FILE:LINE:COL:" then the message.
awk -v q="'" -v os="$os" '
  / panicked at / {
    n = split($0, a, q); name = (n > 2) ? a[2] : "?"
    loc = $0; sub(/.* panicked at /, "", loc); sub(/:$/, "", loc)
    msg = ""; if ((getline line) > 0) msg = line
    gsub(/%/, "%25", msg); gsub(/\r/, "", msg)
    # A title ends at "::" and its properties at ",": escape them in the
    # thread name (engine::tests::x).
    gsub(/%/, "%25", name); gsub(/:/, "%3A", name); gsub(/,/, "%2C", name)
    printf "::error title=%s panic in %s::%s (at %s)\n", os, name, msg, loc
  }' "$out" | head -20

# A test binary that crashed or was killed.
grep -E "process didn't exit successfully" "$out" | head -5 |
  while IFS= read -r l; do echo "::error title=$os: test binary::$l" | esc; done

if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
  {
    echo "## Test failures ($os)"
    echo '```'
    grep -E -A1 '^test .* \.\.\. FAILED$| panicked at |^error: |didn.t exit successfully' "$out" | head -150
    echo '```'
  } >> "$GITHUB_STEP_SUMMARY"
fi
