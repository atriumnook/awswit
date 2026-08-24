#!/usr/bin/env bash
set -euo pipefail

release_tag=${1:?release tag is required}
git check-ref-format "refs/tags/$release_tag"
api_headers=(
  -H "Accept: application/vnd.github+json"
  -H "X-GitHub-Api-Version: 2026-03-10"
)

if ! immutable_release=$(gh api "${api_headers[@]}" \
  "repos/$GITHUB_REPOSITORY/immutable-releases"); then
  echo "Immutable GitHub Releases must be enabled before publishing" >&2
  exit 1
fi
if ! jq -e '.enabled == true' <<<"$immutable_release" >/dev/null; then
  echo "GitHub reported that immutable releases are not enabled" >&2
  exit 1
fi

ruleset_pages=$(gh api --paginate --slurp "${api_headers[@]}" \
  "repos/$GITHUB_REPOSITORY/rulesets?targets=tag&includes_parents=true&per_page=100")
mapfile -t ruleset_ids < <(
  jq -r '.[][] | select(.enforcement == "active") | .id' \
    <<<"$ruleset_pages" | LC_ALL=C sort -nu
)
if [[ "${#ruleset_ids[@]}" -eq 0 ]]; then
  echo "No active tag ruleset protects release tags" >&2
  exit 1
fi

{
  for ruleset_id in "${ruleset_ids[@]}"; do
    gh api "${api_headers[@]}" \
      "repos/$GITHUB_REPOSITORY/rulesets/$ruleset_id?includes_parents=true"
  done
} | jq -s '.' | python3 .github/scripts/validate-release-rulesets.py "$release_tag"
