#!/usr/bin/env python3
"""Verify cargo-dist output and awswit's audited release hardening exactly."""

from __future__ import annotations

import json
import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
CARGO_TOML = ROOT / "Cargo.toml"
CI_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
RELEASE_WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
RELEASE_GATE = ROOT / ".github" / "workflows" / "release-gate.yml"
EXPECTED_ASSETS = ROOT / ".github" / "scripts" / "release-assets.txt"
RELEASE_PLZ = ROOT / "release-plz.toml"


def fail(message: str) -> None:
    raise SystemExit(message)


def remove_marked_block(text: str, begin: str, end: str, replacement: str = "") -> str:
    if text.count(begin) != 1 or text.count(end) != 1:
        fail(f"release marker pair is invalid: {begin.strip()}")
    before, guarded = text.split(begin, 1)
    _, after = guarded.split(end, 1)
    return before + replacement + after


def marked_body(text: str, begin: str, end: str) -> str:
    if text.count(begin) != 1 or text.count(end) != 1:
        fail(f"release marker pair is invalid: {begin.strip()}")
    _, guarded = text.split(begin, 1)
    body, _ = guarded.split(end, 1)
    return body


def verify_custom_release_hardening(hardened: str) -> None:
    generated_shell_lint_blocks = {
        "RUSTUP PATH": '            echo "$HOME/.cargo/bin" >> "$GITHUB_PATH"\n',
        "LOCAL OUTPUT": "".join(
            [
                "          {\n",
                '            echo "paths<<EOF"\n',
                "            dist print-upload-files-from-manifest --manifest dist-manifest.json\n",
                '            echo "EOF"\n',
                '          } >> "$GITHUB_OUTPUT"\n',
            ]
        ),
        "GLOBAL OUTPUT": "".join(
            [
                "          {\n",
                '            echo "paths<<EOF"\n',
                '            jq --raw-output ".upload_files[]" dist-manifest.json\n',
                '            echo "EOF"\n',
                '          } >> "$GITHUB_OUTPUT"\n',
            ]
        ),
    }
    for name, expected in generated_shell_lint_blocks.items():
        begin = f"{'            ' if name == 'RUSTUP PATH' else '          '}# BEGIN AWSWIT GENERATED SHELL LINT ({name})\n"
        end = f"{'            ' if name == 'RUSTUP PATH' else '          '}# END AWSWIT GENERATED SHELL LINT ({name})\n"
        if marked_body(hardened, begin, end) != expected:
            fail(f"generated shell lint block drifted: {name.lower()}")

    policy_blocks = {
        "PLAN": "".join(
            [
                "      - name: Verify immutable releases and protected release tags\n",
                "        if: ${{ inputs.tag != 'dry-run' }}\n",
                "        env:\n",
                "          RELEASE_TAG: ${{ inputs.tag }}\n",
                '        run: bash .github/scripts/check-release-repository-policy.sh "$RELEASE_TAG"\n',
            ]
        ),
        "HOST": "".join(
            [
                "      - name: Reverify immutable releases and protected release tags\n",
                "        env:\n",
                "          RELEASE_TAG: ${{ needs.plan.outputs.tag }}\n",
                '        run: bash .github/scripts/check-release-repository-policy.sh "$RELEASE_TAG"\n',
            ]
        ),
    }
    for phase, expected in policy_blocks.items():
        begin = f"      # BEGIN AWSWIT RELEASE REPOSITORY POLICY ({phase})\n"
        end = f"      # END AWSWIT RELEASE REPOSITORY POLICY ({phase})\n"
        if marked_body(hardened, begin, end) != expected:
            fail(f"release repository policy {phase.lower()} block drifted")

    final_begin = "      # BEGIN AWSWIT FINAL ASSET VALIDATION\n"
    final_end = "      # END AWSWIT FINAL ASSET VALIDATION\n"
    expected_final = "".join(
        [
            "      - name: Verify exact final release asset set\n",
            "        run: python3 .github/scripts/validate-release-assets.py --phase final artifacts\n",
        ]
    )
    if marked_body(hardened, final_begin, final_end) != expected_final:
        fail("final release asset validation block drifted")
    if hardened.index(final_end) > hardened.index("      - name: Attest\n"):
        fail("final asset validation must run before attestation")

    dependency = marked_body(
        hardened,
        "      # BEGIN AWSWIT RELEASE GATE GLOBAL DEPENDENCY\n",
        "      # END AWSWIT RELEASE GATE GLOBAL DEPENDENCY\n",
    )
    if dependency != "      - build-global-artifacts\n":
        fail("release gate must depend on global artifacts")

    transaction = marked_body(
        hardened,
        "      # BEGIN AWSWIT TRANSACTIONAL RELEASE STEP\n",
        "      # END AWSWIT TRANSACTIONAL RELEASE STEP\n",
    )
    required_fragments = {
        '          bash .github/scripts/check-release-repository-policy.sh "$RELEASE_TAG"\n': 1,
        '          git check-ref-format "refs/tags/$RELEASE_TAG"\n': 1,
        '          owned_tag=false\n': 2,
        '          owned_release_id=""\n': 2,
        '          gh api --method POST "repos/$GITHUB_REPOSITORY/git/refs" \\\n': 1,
        '            -f ref="$tag_ref" \\\n': 1,
        '            -f sha="$RELEASE_COMMIT" >/dev/null\n': 1,
        '          owned_tag=true\n': 1,
        '          test "$(remote_tag_sha)" = "$RELEASE_COMMIT"\n': 2,
        '            \'{tag_name: $tag, name: $title, body: $body, draft: true, prerelease: $prerelease}\')\n': 1,
        '          owned_release_id=$(jq -er \'.id | select(type == "number")\' <<<"$release_response")\n': 1,
        '          gh release upload "$RELEASE_TAG" artifacts/*\n': 1,
        "            --jq '.assets[].name' | LC_ALL=C sort)\n": 1,
        '              release_state=$(gh api \\\n': 1,
        '                "repos/$GITHUB_REPOSITORY/releases/$owned_release_id" 2>/dev/null || true)\n': 1,
        "                '.id == $id and .tag_name == $tag and .draft == true' \\\n": 1,
        '                "repos/$GITHUB_REPOSITORY/releases/$owned_release_id"; then\n': 1,
        '            "repos/$GITHUB_REPOSITORY/releases/$owned_release_id" \\\n': 1,
        '              elif [[ -n "$current_tag_sha" ]]; then\n': 1,
        '          trap \'cleanup_release "$?"\' ERR\n': 1,
        "          trap 'cleanup_release 130' INT\n": 1,
        "          trap 'cleanup_release 143' TERM\n": 1,
    }
    for fragment, expected_count in required_fragments.items():
        if transaction.count(fragment) != expected_count:
            fail(f"transactional release ownership contract drifted: {fragment.strip()}")
    for forbidden in ("--cleanup-tag", "gh release create", ' --target "$RELEASE_COMMIT"'):
        if forbidden in transaction:
            fail(f"unsafe transactional release operation present: {forbidden}")


def verify_release_gate_contract() -> None:
    gate = RELEASE_GATE.read_text(encoding="utf-8")
    ci = CI_WORKFLOW.read_text(encoding="utf-8")
    required_fragments = [
        "  workflow_call:\n",
        "    name: Rust 1.94 and four-shell quality\n",
        "        run: cargo fmt --all -- --check\n",
        "        run: cargo clippy --locked --all-targets --all-features -- -D warnings\n",
        "        run: cargo test --locked --all-targets --all-features\n",
        "          cargo test --locked --release --lib performance_contract\n",
        "    name: Dependency policy and advisories\n",
        "      - repository-contract\n",
        "      - dependency-policy\n",
        "        run: python3 .github/scripts/validate-release-assets.py --phase build artifacts\n",
        "        run: python3 .github/scripts/test-release-ruleset-contract.py\n",
        "        run: bash .github/scripts/test-release-repository-policy.sh\n",
        "        run: bash .github/scripts/test-release-transaction.sh\n",
        "        run: bash -n artifacts/awswit-installer.sh\n",
        '(Resolve-Path "artifacts/awswit-installer.ps1"),\n',
        "          name: artifacts-build-release-gate\n",
        '          expected_hash="cd355dab0b4c02fb59038fef87655550021d07f45f1d82f947a34ef98560abb8"\n',
        '          expected_hash="fb8dbee9f182173e062a64a387b21a0badc6fab8b2abf9294973f012972bf6d8"\n',
    ]
    for fragment in required_fragments:
        if fragment not in gate:
            fail(f"release gate contract drifted: {fragment.strip()}")

    shell_begin = "      # BEGIN AWSWIT FOUR SHELL CONFORMANCE\n"
    shell_end = "      # END AWSWIT FOUR SHELL CONFORMANCE\n"
    if marked_body(ci, shell_begin, shell_end) != marked_body(
        gate, shell_begin, shell_end
    ):
        fail("release gate four-shell conformance steps drifted from ordinary CI")

    for path in (RELEASE_WORKFLOW, RELEASE_GATE):
        for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            match = re.match(r"\s*(?:-\s*)?uses:\s*([^#\s]+)", line)
            if not match:
                continue
            action = match.group(1)
            if action.startswith("./"):
                continue
            if not re.fullmatch(r"[^@\s]+@[0-9a-f]{40}", action):
                fail(f"unpinned release action at {path}:{line_number}: {action}")


def normalized_hardened_workflow(hardened: str) -> str:
    normalized = remove_marked_block(
        hardened,
        "# BEGIN AWSWIT LEAST-PRIVILEGE DEFAULT\n",
        "# END AWSWIT LEAST-PRIVILEGE DEFAULT\n",
        'permissions:\n  "contents": "write"\n',
    )
    normalized = remove_marked_block(
        normalized,
        "# BEGIN AWSWIT RELEASE CONCURRENCY\n",
        "# END AWSWIT RELEASE CONCURRENCY\n\n",
    )
    normalized = remove_marked_block(
        normalized,
        "      # BEGIN AWSWIT RELEASE INPUT GUARD\n",
        "      # END AWSWIT RELEASE INPUT GUARD\n",
    )
    for phase in ("PLAN", "HOST"):
        normalized = remove_marked_block(
            normalized,
            f"      # BEGIN AWSWIT RELEASE REPOSITORY POLICY ({phase})\n",
            f"      # END AWSWIT RELEASE REPOSITORY POLICY ({phase})\n",
        )

    gha = lambda expression: "${{ " + expression + " }}"
    verified_blocks = {
        "PLAN": "\n".join(
            [
                "      - name: Install dist",
                "        # we specify bash to get pipefail; it guards against the `curl` command",
                "        # failing. otherwise `sh` won't catch that `curl` returned non-0",
                "        shell: bash",
                '        run: "curl --proto \'=https\' --tlsv1.2 -LsSf https://github.com/axodotdev/cargo-dist/releases/download/v0.31.0/cargo-dist-installer.sh | sh"',
                "      - name: Cache dist",
                "        uses: actions/upload-artifact@b7c566a772e6b6bfb58ed0dc250532a479d7789f",
                "        with:",
                "          name: cargo-dist-cache",
                "          path: ~/.cargo/bin/dist",
                "",
            ]
        ),
        "BUILD": "\n".join(
            [
                "      - name: Install dist",
                "        run: " + gha("matrix.install_dist.run"),
                "",
            ]
        ),
        "GLOBAL": "\n".join(
            [
                "      - name: Install cached dist",
                "        uses: actions/download-artifact@37930b1c2abaa49bbe596cd826c3c89aef350131",
                "        with:",
                "          name: cargo-dist-cache",
                "          path: ~/.cargo/bin/",
                "      - run: chmod +x ~/.cargo/bin/dist",
                "",
            ]
        ),
        "HOST": "\n".join(
            [
                "      - name: Install cached dist",
                "        uses: actions/download-artifact@37930b1c2abaa49bbe596cd826c3c89aef350131",
                "        with:",
                "          name: cargo-dist-cache",
                "          path: ~/.cargo/bin/",
                "      - run: chmod +x ~/.cargo/bin/dist",
                "",
            ]
        ),
    }
    for name, generated_install in verified_blocks.items():
        normalized = remove_marked_block(
            normalized,
            f"      # BEGIN AWSWIT VERIFIED DIST INSTALL ({name})\n",
            f"      # END AWSWIT VERIFIED DIST INSTALL ({name})\n",
            generated_install,
        )

    normalized = remove_marked_block(
        normalized,
        "      # BEGIN AWSWIT NATIVE ARTIFACT SMOKE\n",
        "      # END AWSWIT NATIVE ARTIFACT SMOKE\n",
    )
    normalized = remove_marked_block(
        normalized,
        "      # BEGIN AWSWIT RELEASE GATE GLOBAL DEPENDENCY\n",
        "      # END AWSWIT RELEASE GATE GLOBAL DEPENDENCY\n",
    )
    normalized = remove_marked_block(
        normalized,
        "      # BEGIN AWSWIT FINAL ASSET VALIDATION\n",
        "      # END AWSWIT FINAL ASSET VALIDATION\n",
    )
    normalized = remove_marked_block(
        normalized,
        "    # BEGIN AWSWIT NO INHERITED RELEASE SECRETS\n",
        "    # END AWSWIT NO INHERITED RELEASE SECRETS\n",
        "    secrets: inherit\n",
    )
    generated_shell_lint_replacements = {
        "RUSTUP PATH": (
            "            ",
            '            echo "$HOME/.cargo/bin" >> $GITHUB_PATH\n',
        ),
        "LOCAL OUTPUT": (
            "          ",
            "".join(
                [
                    '          echo "paths<<EOF" >> "$GITHUB_OUTPUT"\n',
                    "          dist print-upload-files-from-manifest --manifest dist-manifest.json >> \"$GITHUB_OUTPUT\"\n",
                    '          echo "EOF" >> "$GITHUB_OUTPUT"\n',
                ]
            ),
        ),
        "GLOBAL OUTPUT": (
            "          ",
            "".join(
                [
                    '          echo "paths<<EOF" >> "$GITHUB_OUTPUT"\n',
                    '          jq --raw-output ".upload_files[]" dist-manifest.json >> "$GITHUB_OUTPUT"\n',
                    '          echo "EOF" >> "$GITHUB_OUTPUT"\n',
                ]
            ),
        ),
    }
    for name, (indent, replacement) in generated_shell_lint_replacements.items():
        normalized = remove_marked_block(
            normalized,
            f"{indent}# BEGIN AWSWIT GENERATED SHELL LINT ({name})\n",
            f"{indent}# END AWSWIT GENERATED SHELL LINT ({name})\n",
            replacement,
        )

    hardened_host = "\n".join(
        [
            "    # Publishing is fail-closed: every build and verification gate must succeed.",
            "    if: "
            + gha(
                "always() && needs.plan.result == 'success' && needs.plan.outputs.publishing == 'true' && needs.build-global-artifacts.result == 'success' && needs.custom-release-gate.result == 'success' && needs.build-local-artifacts.result == 'success'"
            ),
            "",
        ]
    )
    generated_host = "\n".join(
        [
            '    # Only run if we\'re "publishing", and only if plan, local and global didn\'t fail (skipped is fine)',
            "    if: "
            + gha(
                "always() && needs.plan.result == 'success' && needs.plan.outputs.publishing == 'true' && (needs.build-global-artifacts.result == 'skipped' || needs.build-global-artifacts.result == 'success') && (needs.custom-release-gate.result == 'skipped' || needs.custom-release-gate.result == 'success') && (needs.build-local-artifacts.result == 'skipped' || needs.build-local-artifacts.result == 'success')"
            ),
            "",
        ]
    )
    if normalized.count(hardened_host) != 1:
        fail("fail-closed host condition is missing or duplicated")
    normalized = normalized.replace(hardened_host, generated_host)

    publish_begin = "      # BEGIN AWSWIT TRANSACTIONAL RELEASE STEP\n"
    publish_end = "      # END AWSWIT TRANSACTIONAL RELEASE STEP\n"
    generated_publish = "\n".join(
        [
            '      - name: Create GitHub Release',
            "        env:",
            '          PRERELEASE_FLAG: "'
            + gha(
                "fromJson(steps.host.outputs.manifest).announcement_is_prerelease && '--prerelease' || ''"
            )
            + '"',
            '          ANNOUNCEMENT_TITLE: "'
            + gha("fromJson(steps.host.outputs.manifest).announcement_title")
            + '"',
            '          ANNOUNCEMENT_BODY: "'
            + gha("fromJson(steps.host.outputs.manifest).announcement_github_body")
            + '"',
            '          RELEASE_COMMIT: "' + gha("github.sha") + '"',
            "        run: |",
            "          # Write and read notes from a file to avoid quoting breaking things",
            '          echo "$ANNOUNCEMENT_BODY" > $RUNNER_TEMP/notes.txt',
            "",
            '          gh release create "'
            + gha("needs.plan.outputs.tag")
            + '" --target "$RELEASE_COMMIT" $PRERELEASE_FLAG --title "$ANNOUNCEMENT_TITLE" --notes-file "$RUNNER_TEMP/notes.txt" artifacts/*',
            "",
        ]
    )
    normalized = remove_marked_block(
        normalized, publish_begin, publish_end, generated_publish
    )
    return normalized


def generate_pristine_workflow() -> str:
    hardened = RELEASE_WORKFLOW.read_text()
    cargo = CARGO_TOML.read_text()
    allow_dirty = 'allow-dirty = ["ci"]\n'
    if cargo.count(allow_dirty) != 1:
        fail("cargo-dist allow-dirty contract is missing or duplicated")
    try:
        CARGO_TOML.write_text(cargo.replace(allow_dirty, ""))
        subprocess.run(
            ["dist", "generate", "--mode=ci"], cwd=ROOT, check=True
        )
        return RELEASE_WORKFLOW.read_text()
    finally:
        CARGO_TOML.write_text(cargo)
        RELEASE_WORKFLOW.write_text(hardened)


def verify_plan() -> None:
    result = subprocess.run(
        ["dist", "plan", "--output-format=json"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    plan = json.loads(result.stdout)
    actual_targets = sorted(
        target
        for entry in plan["ci"]["github"]["artifacts_matrix"]["include"]
        for target in entry["targets"]
    )
    expected_targets = sorted(
        [
            "aarch64-apple-darwin",
            "aarch64-unknown-linux-gnu",
            "x86_64-apple-darwin",
            "x86_64-pc-windows-msvc",
            "x86_64-unknown-linux-gnu",
        ]
    )
    if actual_targets != expected_targets:
        fail(f"cargo-dist target plan drifted: {actual_targets!r}")

    expected_assets = sorted(EXPECTED_ASSETS.read_text().splitlines())
    releases = plan.get("releases", [])
    if len(releases) != 1:
        fail("cargo-dist must plan exactly one awswit release")
    actual_assets = sorted(releases[0].get("artifacts", []))
    if actual_assets != expected_assets:
        fail(f"cargo-dist release asset plan drifted: {actual_assets!r}")


def verify_binary_only_release_ownership() -> None:
    def boolean_table(path: Path, section: str) -> dict[str, bool]:
        current = ""
        values: dict[str, bool] = {}
        for raw_line in path.read_text(encoding="utf-8").splitlines():
            line = raw_line.split("#", 1)[0].strip()
            if line.startswith("[") and line.endswith("]"):
                current = line[1:-1].strip()
                continue
            if current != section:
                continue
            match = re.fullmatch(r"([A-Za-z0-9_-]+)\s*=\s*(true|false)", line)
            if match:
                name, value = match.groups()
                if name in values:
                    fail(f"duplicate boolean {name!r} in [{section}]")
                values[name] = value == "true"
        return values

    package = boolean_table(CARGO_TOML, "package")
    package_dist = boolean_table(CARGO_TOML, "package.metadata.dist")
    if package.get("publish") is not False or package_dist.get("dist") is not True:
        fail("Cargo.toml must disable registry publication and opt in to cargo-dist")

    workspace = boolean_table(RELEASE_PLZ, "workspace")
    expected = {
        "publish": False,
        "git_only": True,
        "git_tag_enable": False,
        "git_release_enable": False,
    }
    actual = {name: workspace.get(name) for name in expected}
    if actual != expected:
        fail(f"release-plz binary-only ownership contract drifted: {actual!r}")


def main() -> None:
    version = subprocess.run(
        ["dist", "--version"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()
    if version != "cargo-dist 0.31.0":
        fail(f"unexpected cargo-dist version: {version}")

    hardened = RELEASE_WORKFLOW.read_text()
    verify_custom_release_hardening(hardened)
    verify_release_gate_contract()
    generated = generate_pristine_workflow()
    if normalized_hardened_workflow(hardened) != generated:
        fail("release.yml drifted from cargo-dist output")
    verify_binary_only_release_ownership()
    verify_plan()


if __name__ == "__main__":
    main()
