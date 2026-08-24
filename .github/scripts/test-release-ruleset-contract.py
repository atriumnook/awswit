#!/usr/bin/env python3
"""Positive and negative fixtures for release tag ruleset validation."""

from __future__ import annotations

import importlib.util
import unittest
from copy import deepcopy
from pathlib import Path


SCRIPT = Path(__file__).with_name("validate-release-rulesets.py")
SPEC = importlib.util.spec_from_file_location("release_ruleset_contract", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load release ruleset validator")
contract = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(contract)


def valid_ruleset() -> dict:
    return {
        "id": 42,
        "target": "tag",
        "enforcement": "active",
        "bypass_actors": [],
        "conditions": {
            "ref_name": {
                "include": ["refs/tags/v*"],
                "exclude": [],
            }
        },
        "rules": [{"type": "update"}, {"type": "deletion"}],
    }


class ReleaseRulesetContractTests(unittest.TestCase):
    def assert_rejected(self, mutate) -> None:
        ruleset = valid_ruleset()
        mutate(ruleset)
        with self.assertRaises(contract.ContractError):
            contract.protects_release_tag([ruleset], "v1.2.3")

    def test_active_non_bypass_update_and_delete_rules_are_accepted(self) -> None:
        contract.protects_release_tag([valid_ruleset()], "v1.2.3")

    def test_protections_can_be_aggregated_across_rulesets(self) -> None:
        update = valid_ruleset()
        update["rules"] = [{"type": "update"}]
        deletion = deepcopy(update)
        deletion["id"] = 43
        deletion["rules"] = [{"type": "deletion"}]
        contract.protects_release_tag([update, deletion], "v1.2.3")

    def test_inactive_ruleset_is_rejected(self) -> None:
        self.assert_rejected(lambda ruleset: ruleset.update(enforcement="disabled"))

    def test_matching_exclusion_is_rejected(self) -> None:
        self.assert_rejected(
            lambda ruleset: ruleset["conditions"]["ref_name"].update(
                exclude=["refs/tags/v1.*"]
            )
        )

    def test_bypass_actor_is_rejected(self) -> None:
        self.assert_rejected(
            lambda ruleset: ruleset.update(
                bypass_actors=[{"actor_id": 1, "actor_type": "RepositoryRole"}]
            )
        )

    def test_missing_update_or_delete_rule_is_rejected(self) -> None:
        self.assert_rejected(lambda ruleset: ruleset.update(rules=[{"type": "update"}]))

    def test_wildcard_does_not_cross_a_tag_path_separator(self) -> None:
        self.assertFalse(contract.matches_ref("refs/tags/v*", "refs/tags/v1/nested"))


if __name__ == "__main__":
    unittest.main()
