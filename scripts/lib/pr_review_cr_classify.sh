#!/usr/bin/env bash
# Classify CodeRabbit commit-status for gate 1 of check_pr_reviews.sh.
#
# Usage (sourced):
#   classify_coderabbit_head_status <found> <state> <description>
#   Sets: CR_CLASSIFY_OUTCOME = pass | soft_pass | hold
#         CR_CLASSIFY_REASON  = human-readable detail (empty for pass)
#
# Outcomes:
#   pass      — description is "Review completed" (case-insensitive) and state=success
#   soft_pass — GitHub rate-limited / quota unavailable *after* a review was requested
#               (NOT a completed review), when reset is >10 minutes OR the reset is
#               not reliably parsable from the status description. CI cannot see local
#               CR; agents MUST still confirm either (a) the other side completed
#               cleanly, or (b) BOTH local and GitHub are rate-limited with reported
#               resets >10m (or long/unknown on both) before merge. Soft-passes for
#               success and failure states — CR may report rate-limit either way.
#               Soft-pass ≠ on-demand skip.
#   hold      — pending / missing / error / unexpected / non-completed success /
#               on-demand or label skips (never requested — same as missing); OR
#               rate-limited with a reliably parsed reset of ≤10 minutes (wait/retry)
#
# Reset parsing (conservative — prefer soft-pass over brittle false reds):
#   Looks for phrases like "Next included review available in N minutes" /
#   "available in N minutes" / "in N minutes" / "in N hours". Only HOLD when an
#   integer minute (or hour→minute) value is extracted and that value is ≤10.
#   Unparsable rate-limit descriptions soft-pass with a loud warning.

# Extract reset minutes from a CodeRabbit status description when reliable.
# Prints an integer minute count on stdout and returns 0 when parsed; else return 1.
parse_coderabbit_reset_minutes() {
  local desc="${1:-}"
  local desc_lc
  local n
  desc_lc="$(printf '%s' "$desc" | tr '[:upper:]' '[:lower:]')"

  # Prefer explicit "available in N …" (CodeRabbit: "Next included review available in N minutes").
  if [[ "$desc_lc" =~ available[[:space:]]+in[[:space:]]+([0-9]+)[[:space:]]*minutes? ]]; then
    n="${BASH_REMATCH[1]}"
    printf '%s\n' "$n"
    return 0
  fi
  if [[ "$desc_lc" =~ available[[:space:]]+in[[:space:]]+([0-9]+)[[:space:]]*hours? ]]; then
    n="${BASH_REMATCH[1]}"
    # Saturate large hour values; 1h+ is always >10m for the hold threshold.
    if (( n > 1000 )); then
      printf '60000\n'
    else
      printf '%s\n' "$((n * 60))"
    fi
    return 0
  fi
  # Fallback: "… in N minutes" without requiring "available".
  if [[ "$desc_lc" =~ (^|[^a-z])in[[:space:]]+([0-9]+)[[:space:]]*minutes?([^a-z]|$) ]]; then
    n="${BASH_REMATCH[2]}"
    printf '%s\n' "$n"
    return 0
  fi
  if [[ "$desc_lc" =~ (^|[^a-z])in[[:space:]]+([0-9]+)[[:space:]]*hours?([^a-z]|$) ]]; then
    n="${BASH_REMATCH[2]}"
    if (( n > 1000 )); then
      printf '60000\n'
    else
      printf '%s\n' "$((n * 60))"
    fi
    return 0
  fi
  return 1
}

classify_coderabbit_head_status() {
  local found="${1:-}"
  local state="${2:-}"
  local desc="${3:-}"
  local desc_lc
  local reset_mins=""
  desc_lc="$(printf '%s' "$desc" | tr '[:upper:]' '[:lower:]')"

  CR_CLASSIFY_OUTCOME="hold"
  CR_CLASSIFY_REASON=""

  if [[ "$found" != "true" ]]; then
    CR_CLASSIFY_REASON="CodeRabbit has not reported a commit status on HEAD yet (pending / not started / on-demand not requested)."
    return 0
  fi
  if [[ "$state" == "pending" ]]; then
    CR_CLASSIFY_REASON="CodeRabbit is still pending: ${desc:-pending}"
    return 0
  fi
  # On-demand / label skip: hard-fail (hold) BEFORE rate-limit soft-pass so a
  # description that somehow mentions both cannot soft-pass. Examples:
  #   "Review skipped: excluded by label configuration"
  #   "Review skipped: on demand"
  #   "Review skipped: …"
  # Lesson: PRs #278/#279 soft-passed these and merged without requesting CR.
  if [[ "$desc_lc" == *skip* || "$desc_lc" == *"excluded by label"* || "$desc_lc" == *"on demand"* ]]; then
    CR_CLASSIFY_REASON="CodeRabbit on-demand skip on HEAD (review never requested): ${desc:-no description}. Request '@coderabbitai full review' or label coderabbit-review."
    return 0
  fi
  # Rate-limit / quota after a request. Soft-pass when reset >10m or unparsable;
  # HOLD when a reliable parse yields ≤10 minutes (agents/humans should wait).
  # Intentionally matches failure as well as success — CodeRabbit has reported
  # "Review rate limited" under both states.
  if [[ "$desc_lc" == *rate*limit* || "$desc_lc" == *quota* || "$desc_lc" == *"too many request"* ]]; then
    if reset_mins="$(parse_coderabbit_reset_minutes "$desc")"; then
      if [[ "$reset_mins" =~ ^[0-9]+$ ]] && (( reset_mins <= 10 )); then
        CR_CLASSIFY_OUTCOME="hold"
        CR_CLASSIFY_REASON="rate-limited with parsed reset ≤10m (${reset_mins}m) — wait/retry (do not soft-pass-merge yet): ${desc:-no description}"
        return 0
      fi
      CR_CLASSIFY_OUTCOME="soft_pass"
      CR_CLASSIFY_REASON="rate-limited (>10m parsed: ${reset_mins}m)"
      return 0
    fi
    CR_CLASSIFY_OUTCOME="soft_pass"
    CR_CLASSIFY_REASON="rate-limited (reset unparsable — soft-pass with warning)"
    return 0
  fi
  if [[ "$state" == "failure" || "$state" == "error" ]]; then
    CR_CLASSIFY_REASON="CodeRabbit status is ${state}: ${desc:-no description}"
    return 0
  fi
  if [[ "$state" != "success" ]]; then
    CR_CLASSIFY_REASON="CodeRabbit status is unexpected (${state}: ${desc:-no description})."
    return 0
  fi
  if [[ "$desc_lc" != "review completed" ]]; then
    CR_CLASSIFY_REASON="CodeRabbit is not a completed review (status=\"${desc:-none}\"; state=${state}). Waiting for \"Review completed\"."
    return 0
  fi
  CR_CLASSIFY_OUTCOME="pass"
  CR_CLASSIFY_REASON=""
  return 0
}
