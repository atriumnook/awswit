#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
test_root=$(mktemp -d)
trap 'rm -r -- "$test_root"' EXIT

run_case() {
  local scenario=$1
  local expected_status=$2
  set +e
  (
    cd "$repo_root"
    export GITHUB_REPOSITORY="owner/awswit"
    export MOCK_SCENARIO="$scenario"

    git() {
      if [[ "$1" == "check-ref-format" ]]; then
        return 0
      fi
      return 90
    }

    gh() {
      local endpoint=${!#}
      case "$endpoint" in
        repos/*/immutable-releases)
          if [[ "$MOCK_SCENARIO" == "immutable_disabled" ]]; then
            return 1
          fi
          printf '{"enabled":true,"enforced_by_owner":false}\n'
          ;;
        *'/rulesets?targets=tag&includes_parents=true&per_page=100')
          if [[ "$MOCK_SCENARIO" == "no_ruleset" ]]; then
            printf '[[]]\n'
          else
            printf '[[{"id":42,"enforcement":"active"}]]\n'
          fi
          ;;
        *'/rulesets/42?includes_parents=true')
          local bypass='[]'
          if [[ "$MOCK_SCENARIO" == "bypass_actor" ]]; then
            bypass='[{"actor_id":1,"actor_type":"RepositoryRole"}]'
          fi
          printf '%s\n' "{\"id\":42,\"target\":\"tag\",\"enforcement\":\"active\",\"bypass_actors\":$bypass,\"conditions\":{\"ref_name\":{\"include\":[\"refs/tags/v*\"],\"exclude\":[]}},\"rules\":[{\"type\":\"update\"},{\"type\":\"deletion\"}]}"
          ;;
        *)
          printf 'unexpected policy API endpoint: %s\n' "$endpoint" >&2
          return 91
          ;;
      esac
    }

    # shellcheck source=/dev/null
    source .github/scripts/check-release-repository-policy.sh v1.2.3
  ) >"$test_root/$scenario.stdout" 2>"$test_root/$scenario.stderr"
  local status=$?
  set -e
  if [[ "$expected_status" == "success" ]]; then
    test "$status" -eq 0 || {
      cat "$test_root/$scenario.stderr" >&2
      return 1
    }
  else
    test "$status" -ne 0
  fi
}

run_case valid success
run_case immutable_disabled failure
run_case no_ruleset failure
run_case bypass_actor failure

echo "release repository policy fixtures passed"
