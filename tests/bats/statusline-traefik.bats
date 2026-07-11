#!/usr/bin/env bats
# Tests for statusline-command.sh Traefik API jq filter (Design C)
#
# Current filter (pre-fix — does NOT filter by branch):
#   jq -r --arg proj "$proj" '
#     [.[] |
#     select(.name | endswith("--" + $proj + "@file")) |
#     select(.serverStatus | to_entries | map(.value == "UP") | any) |
#     .name | split("-")[0]] | join(" ")'
#
# Target filter (post-fix — branch-aware):
#   jq -r --arg proj "$proj" --arg branch "$branch" '
#     [.[] |
#     select(.name | test("^p[0-9]+-" + $branch + "--" + $proj + "@file$")) |
#     select(.serverStatus | to_entries | map(.value == "UP") | any) |
#     .name | split("-")[0]] | join(" ")'
#
# These tests are written for the TARGET filter (TDD Red phase).
# They FAIL against the current filter and PASS after the fix.

JQ_FILTER_CURRENT='
[.[] |
select(.name | endswith("--" + $proj + "@file")) |
select(.serverStatus | to_entries | map(.value == "UP") | any) |
.name | split("-")[0]] | join(" ")'

JQ_FILTER_TARGET='
[.[] |
select(.name | test("^p[0-9]+-" + $branch + "--" + $proj + "@file$")) |
select(.serverStatus | to_entries | map(.value == "UP") | any) |
.name | split("-")[0]] | join(" ")'

MOCK_TWO_BRANCHES='[
  {"name":"p8080-main--proj@file","serverStatus":{"http://198.51.100.2:8080":"UP"}},
  {"name":"p8080-feature-foo--proj@file","serverStatus":{"http://198.51.100.3:8080":"UP"}}
]'

MOCK_NON_UP='[
  {"name":"p8080-main--proj@file","serverStatus":{"http://198.51.100.2:8080":"UP"}},
  {"name":"p9090-main--proj@file","serverStatus":{"http://198.51.100.2:9090":"DOWN"}}
]'

# ---------------------------------------------------------------------------
# Cycle 1: filter matches only current branch
# ---------------------------------------------------------------------------
@test "jq filter matches only current branch" {
    result=$(printf '%s' "$MOCK_TWO_BRANCHES" \
        | jq -r --arg proj "proj" --arg branch "main" "$JQ_FILTER_TARGET")
    [ "$result" = "p8080" ]
}

# ---------------------------------------------------------------------------
# Cycle 2: filter excludes other branch links
# ---------------------------------------------------------------------------
@test "jq filter excludes other branch" {
    result=$(printf '%s' "$MOCK_TWO_BRANCHES" \
        | jq -r --arg proj "proj" --arg branch "feature-foo" "$JQ_FILTER_TARGET")
    [ "$result" = "p8080" ]
}

# ---------------------------------------------------------------------------
# Cycle 3: filter excludes non-UP services
# ---------------------------------------------------------------------------
@test "jq filter excludes non-UP services" {
    result=$(printf '%s' "$MOCK_NON_UP" \
        | jq -r --arg proj "proj" --arg branch "main" "$JQ_FILTER_TARGET")
    [ "$result" = "p8080" ]
}
