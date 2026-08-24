#!/usr/bin/env python3
"""Validate active GitHub tag rulesets needed for a release transaction."""

from __future__ import annotations

import argparse
import fnmatch
import json
import sys
from typing import Any


class ContractError(RuntimeError):
    """The repository rulesets do not protect release tags."""


def matches_ref(pattern: str, ref_name: str) -> bool:
    if pattern == "~ALL":
        return True
    if "\\" in pattern:
        return False
    pattern_parts = pattern.split("/")
    ref_parts = ref_name.split("/")
    if len(pattern_parts) != len(ref_parts):
        # GitHub uses File::FNM_PATHNAME, so '*' cannot cross '/'. Rejecting
        # recursive '**/' patterns is a conservative false-negative.
        return False
    return all(
        fnmatch.fnmatchcase(ref_part, pattern_part)
        for pattern_part, ref_part in zip(pattern_parts, ref_parts)
    )


def protects_release_tag(rulesets: Any, release_tag: str) -> None:
    if not isinstance(rulesets, list):
        raise ContractError("ruleset response must be a JSON array")
    ref_name = f"refs/tags/{release_tag}"
    protected_rules: set[str] = set()

    for ruleset in rulesets:
        if not isinstance(ruleset, dict):
            continue
        if ruleset.get("target") != "tag" or ruleset.get("enforcement") != "active":
            continue
        # A workflow cleanup bypass would also permit post-release tag mutation.
        # Prefer an orphaned correct-SHA tag on failure over weakening immutability.
        if ruleset.get("bypass_actors") != []:
            continue
        conditions = ruleset.get("conditions")
        if not isinstance(conditions, dict):
            continue
        ref_condition = conditions.get("ref_name")
        if not isinstance(ref_condition, dict):
            continue
        includes = ref_condition.get("include")
        excludes = ref_condition.get("exclude")
        if not isinstance(includes, list) or not isinstance(excludes, list):
            continue
        if not all(isinstance(pattern, str) for pattern in includes + excludes):
            continue
        if not any(matches_ref(pattern, ref_name) for pattern in includes):
            continue
        if any(matches_ref(pattern, ref_name) for pattern in excludes):
            continue
        rules = ruleset.get("rules")
        if not isinstance(rules, list):
            continue
        protected_rules.update(
            rule["type"]
            for rule in rules
            if isinstance(rule, dict) and isinstance(rule.get("type"), str)
        )

    missing = {"deletion", "update"} - protected_rules
    if missing:
        raise ContractError(
            "release tag requires active, non-bypass rulesets that restrict "
            f"update and deletion; missing={sorted(missing)!r}"
        )


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("release_tag")
    args = parser.parse_args()
    try:
        rulesets = json.load(sys.stdin)
        protects_release_tag(rulesets, args.release_tag)
    except (ContractError, json.JSONDecodeError) as error:
        raise SystemExit(f"release tag ruleset contract failed: {error}") from error


if __name__ == "__main__":
    main()
