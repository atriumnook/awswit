#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
test_root=$(mktemp -d)
trap 'rm -r -- "$test_root"' EXIT
transaction="$test_root/transaction.sh"

awk '
  /# BEGIN AWSWIT TRANSACTIONAL RELEASE STEP/ { in_block = 1 }
  in_block && /^        run: \|$/ { in_script = 1; next }
  /# END AWSWIT TRANSACTIONAL RELEASE STEP/ { exit }
  in_script { sub(/^          /, ""); print }
' "$repo_root/.github/workflows/release.yml" > "$transaction"
bash -n "$transaction"

assert_logged() {
  local log=$1
  local expected=$2
  grep -Fxq -- "$expected" "$log" || {
    echo "expected transaction event was not logged: $expected" >&2
    return 1
  }
}

assert_not_logged() {
  local log=$1
  local unexpected=$2
  if grep -Fxq -- "$unexpected" "$log"; then
    echo "unsafe transaction event was logged: $unexpected" >&2
    return 1
  fi
}

run_case() {
  local scenario=$1
  local expected_status=$2
  local case_root="$test_root/$scenario"
  local state="$case_root/state"
  local log="$case_root/events.log"
  mkdir -p "$case_root/artifacts" "$case_root/tmp" "$state"
  printf 'archive\n' > "$case_root/artifacts/awswit.tar.xz"
  printf 'installer\n' > "$case_root/artifacts/awswit-installer.sh"
  : > "$log"
  if [[ "$scenario" == "preexisting_release" ]]; then
    : > "$state/release-exists"
  fi

  set +e
  (
    cd "$case_root"
    export ANNOUNCEMENT_BODY=$'release notes\nwith a second line'
    export ANNOUNCEMENT_TITLE="awswit fixture"
    export GITHUB_REPOSITORY="owner/awswit"
    export MOCK_LOG="$log"
    export MOCK_SCENARIO="$scenario"
    export MOCK_STATE="$state"
    export PRERELEASE_FLAG=""
    export RELEASE_COMMIT="0123456789abcdef0123456789abcdef01234567"
    export RELEASE_TAG="v1.2.3"
    export RUNNER_TEMP="$case_root/tmp"

    bash() {
      if [[ "$1" == ".github/scripts/check-release-repository-policy.sh" ]]; then
        printf '%s\n' "POLICY_CHECK" >> "$MOCK_LOG"
        return 0
      fi
      command bash "$@"
    }

    gh() {
      if [[ "$1" == "release" && "$2" == "view" ]]; then
        if [[ " $* " == *" --json "* ]]; then
          find artifacts -maxdepth 1 -type f -printf '%f\n' | LC_ALL=C sort
          return 0
        fi
        test -e "$MOCK_STATE/release-exists"
        return
      fi
      if [[ "$1" == "release" && "$2" == "upload" ]]; then
        printf '%s\n' "UPLOAD" >> "$MOCK_LOG"
        if [[ "$MOCK_SCENARIO" == "upload_failure" || "$MOCK_SCENARIO" == "changed_tag" ]]; then
          return 1
        fi
        if [[ "$MOCK_SCENARIO" == "signal_int" ]]; then
          kill -INT "$BASHPID"
          return 99
        fi
        if [[ "$MOCK_SCENARIO" == "signal_term" ]]; then
          kill -TERM "$BASHPID"
          return 99
        fi
        return 0
      fi
      if [[ "$1" == "api" && "$2" == repos/*/releases/123 ]]; then
        local draft=true
        if [[ -e "$MOCK_STATE/published" ]]; then
          draft=false
        fi
        printf '{"id":123,"tag_name":"v1.2.3","draft":%s}\n' "$draft"
        return 0
      fi
      if [[ "$1" != "api" || "$2" != "--method" ]]; then
        return 90
      fi

      local method=$3
      local endpoint=$4
      case "$method:$endpoint" in
        POST:*/git/refs)
          printf '%s\n' "POST_REF" >> "$MOCK_LOG"
          if [[ "$MOCK_SCENARIO" == "preexisting_tag" ]]; then
            return 1
          fi
          : > "$MOCK_STATE/tag-exists"
          printf '{"ref":"refs/tags/v1.2.3"}\n'
          ;;
        POST:*/releases)
          printf '%s\n' "POST_RELEASE" >> "$MOCK_LOG"
          if [[ "$MOCK_SCENARIO" == "release_race" ]]; then
            : > "$MOCK_STATE/release-exists"
            return 1
          fi
          : > "$MOCK_STATE/release-exists"
          printf '{"id":123}\n'
          ;;
        DELETE:*/releases/123)
          printf '%s\n' "DELETE_RELEASE" >> "$MOCK_LOG"
          rm -f -- "$MOCK_STATE/release-exists"
          ;;
        DELETE:*/git/refs/tags/v1.2.3)
          # The production policy requires a no-bypass deletion rule, so the
          # workflow may request cleanup but GitHub must preserve the tag.
          printf '%s\n' "DELETE_REF_ATTEMPT" >> "$MOCK_LOG"
          return 1
          ;;
        PATCH:*/releases/123)
          printf '%s\n' "PUBLISH_RELEASE" >> "$MOCK_LOG"
          if [[ "$MOCK_SCENARIO" == "publish_response_failure" ]]; then
            : > "$MOCK_STATE/published"
            return 1
          fi
          ;;
        *)
          printf 'unexpected gh api call: %s %s\n' "$method" "$endpoint" >&2
          return 91
          ;;
      esac
    }

    git() {
      if [[ "$1" == "check-ref-format" ]]; then
        return 0
      fi
      if [[ "$1" != "ls-remote" ]]; then
        return 92
      fi
      test -e "$MOCK_STATE/tag-exists" || return 2
      local count=0
      if [[ -e "$MOCK_STATE/ref-reads" ]]; then
        count=$(<"$MOCK_STATE/ref-reads")
      fi
      count=$((count + 1))
      printf '%s\n' "$count" > "$MOCK_STATE/ref-reads"
      if [[ "$MOCK_SCENARIO" == "changed_tag" && "$count" -gt 1 ]]; then
        printf '%s\t%s\n' "ffffffffffffffffffffffffffffffffffffffff" "refs/tags/$RELEASE_TAG"
      else
        printf '%s\t%s\n' "$RELEASE_COMMIT" "refs/tags/$RELEASE_TAG"
      fi
    }

    # shellcheck source=/dev/null
    source "$transaction"
  )
  local status=$?
  set -e
  if [[ "$expected_status" == "success" ]]; then
    test "$status" -eq 0
  elif [[ "$expected_status" =~ ^[0-9]+$ ]]; then
    test "$status" -eq "$expected_status"
  else
    test "$status" -ne 0
  fi

  case "$scenario" in
    success)
      assert_logged "$log" "POLICY_CHECK"
      assert_logged "$log" "POST_REF"
      assert_logged "$log" "POST_RELEASE"
      assert_logged "$log" "UPLOAD"
      assert_logged "$log" "PUBLISH_RELEASE"
      assert_not_logged "$log" "DELETE_RELEASE"
      assert_not_logged "$log" "DELETE_REF_ATTEMPT"
      ;;
    preexisting_release)
      assert_not_logged "$log" "POST_REF"
      assert_not_logged "$log" "DELETE_REF_ATTEMPT"
      ;;
    preexisting_tag)
      assert_logged "$log" "POST_REF"
      assert_not_logged "$log" "POST_RELEASE"
      assert_not_logged "$log" "DELETE_REF_ATTEMPT"
      ;;
    release_race)
      assert_logged "$log" "POST_REF"
      assert_logged "$log" "POST_RELEASE"
      assert_not_logged "$log" "DELETE_RELEASE"
      assert_not_logged "$log" "DELETE_REF_ATTEMPT"
      ;;
    upload_failure|signal_int|signal_term)
      assert_logged "$log" "DELETE_RELEASE"
      assert_logged "$log" "DELETE_REF_ATTEMPT"
      test -e "$state/tag-exists"
      ;;
    changed_tag)
      assert_logged "$log" "DELETE_RELEASE"
      assert_not_logged "$log" "DELETE_REF_ATTEMPT"
      ;;
    publish_response_failure)
      assert_logged "$log" "PUBLISH_RELEASE"
      assert_not_logged "$log" "DELETE_RELEASE"
      assert_not_logged "$log" "DELETE_REF_ATTEMPT"
      ;;
  esac
}

run_case success success
run_case preexisting_release failure
run_case preexisting_tag failure
run_case release_race failure
run_case upload_failure failure
run_case changed_tag failure
run_case publish_response_failure failure
run_case signal_int 130
run_case signal_term 143

echo "release transaction ownership fixtures passed"
