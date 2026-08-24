#!/usr/bin/env python3
"""Fail-closed validation for cargo-dist build and final release asset sets."""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import stat
import tarfile
import zipfile
from pathlib import Path, PurePosixPath
from xml.etree import ElementTree


ROOT = Path(__file__).resolve().parents[2]
CANONICAL_ASSETS = ROOT / ".github" / "scripts" / "release-assets.txt"
BUILD_MANIFESTS = {
    "aarch64-apple-darwin-dist-manifest.json",
    "aarch64-unknown-linux-gnu-dist-manifest.json",
    "global-dist-manifest.json",
    "x86_64-apple-darwin-dist-manifest.json",
    "x86_64-pc-windows-msvc-dist-manifest.json",
    "x86_64-unknown-linux-gnu-dist-manifest.json",
}
FINAL_ADDITIONS = {"awswit.cdx.xml", "dist-manifest.json"}
ARCHIVES = {
    "awswit-aarch64-apple-darwin.tar.xz",
    "awswit-aarch64-unknown-linux-gnu.tar.xz",
    "awswit-x86_64-apple-darwin.tar.xz",
    "awswit-x86_64-pc-windows-msvc.zip",
    "awswit-x86_64-unknown-linux-gnu.tar.xz",
    "source.tar.gz",
}
SOURCE_FILE_PATHS = {
    ".github/scripts/check-local-markdown-links.py",
    ".github/scripts/check-release-contract.py",
    ".github/scripts/check-release-repository-policy.sh",
    ".github/scripts/release-assets.txt",
    ".github/scripts/test-release-asset-contract.py",
    ".github/scripts/test-release-repository-policy.sh",
    ".github/scripts/test-release-ruleset-contract.py",
    ".github/scripts/test-release-transaction.sh",
    ".github/scripts/validate-release-assets.py",
    ".github/scripts/validate-release-rulesets.py",
    ".github/workflows/ci.yml",
    ".github/workflows/release-gate.yml",
    ".github/workflows/release-plz.yml",
    ".github/workflows/release.yml",
    ".gitignore",
    "CHANGELOG.md",
    "Cargo.lock",
    "Cargo.toml",
    "LICENSE",
    "README.md",
    "README_ja.md",
    "deny.toml",
    "docs/README.md",
    "docs/design/architecture.md",
    "docs/design/specification.md",
    "docs/operations/runbook.md",
    "docs/requirements/competitive-research.md",
    "docs/requirements/product-requirements.md",
    "release-plz.toml",
    "rust-toolchain.toml",
    "rustfmt.toml",
    "src/activation/mod.rs",
    "src/application.rs",
    "src/catalog/mod.rs",
    "src/catalog/parser.rs",
    "src/catalog/tests.rs",
    "src/cli/args.rs",
    "src/cli/mod.rs",
    "src/error.rs",
    "src/history/mod.rs",
    "src/history/storage.rs",
    "src/init/bash.sh",
    "src/init/fish.fish",
    "src/init/powershell.ps1",
    "src/init/zsh.sh",
    "src/lib.rs",
    "src/main.rs",
    "src/output.rs",
    "src/process.rs",
    "src/safety.rs",
    "src/text_safety.rs",
    "src/tui/mod.rs",
    "src/tui/render.rs",
    "src/tui/state.rs",
    "src/tui/terminal.rs",
    "tests/activation_protocol_vectors.tsv",
    "tests/cli_contract.rs",
    "tests/completion_profiles.ini",
}
HEX_SHA256 = re.compile(r"[0-9a-f]{64}")
CYCLONEDX_NAMESPACE = re.compile(
    r"(?:https?://|urn:)cyclonedx\.org/schema/bom/1\.[0-9]+"
)
MAX_ARCHIVE_MEMBERS = 8_192
MAX_ARCHIVE_UNCOMPRESSED_BYTES = 512 * 1024 * 1024


class ContractError(RuntimeError):
    """A release asset set violated the publication contract."""


def canonical_assets() -> set[str]:
    names = CANONICAL_ASSETS.read_text(encoding="utf-8").splitlines()
    if names != sorted(names) or len(names) != len(set(names)) or not names:
        raise ContractError("canonical release asset list must be non-empty, sorted, and unique")
    return set(names)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def validate_flat_exact_set(directory: Path, expected: set[str]) -> None:
    if not directory.is_dir():
        raise ContractError(f"asset directory does not exist: {directory}")
    entries = list(directory.iterdir())
    invalid = sorted(entry.name for entry in entries if entry.is_symlink() or not entry.is_file())
    if invalid:
        raise ContractError(f"release assets must be flat regular files: {invalid!r}")
    actual = {entry.name for entry in entries}
    if actual != expected:
        missing = sorted(expected - actual)
        unexpected = sorted(actual - expected)
        raise ContractError(
            f"release asset set mismatch; missing={missing!r}, unexpected={unexpected!r}"
        )
    empty = sorted(name for name in actual if directory.joinpath(name).stat().st_size == 0)
    if empty:
        raise ContractError(f"release assets must be non-empty: {empty!r}")


def parse_checksum_line(line: str, expected_name: str | None) -> tuple[str, str | None]:
    fields = line.split()
    if not fields or len(fields) > 2 or not HEX_SHA256.fullmatch(fields[0]):
        raise ContractError(f"invalid SHA-256 record: {line!r}")
    name = None
    if len(fields) == 2:
        name = fields[1].removeprefix("*")
        if PurePosixPath(name).name != name:
            raise ContractError(f"checksum record contains a path: {name!r}")
    if expected_name is not None and name not in (None, expected_name):
        raise ContractError(
            f"checksum record names {name!r}, expected {expected_name!r}"
        )
    return fields[0], name


def validate_checksums(directory: Path) -> None:
    for archive in sorted(ARCHIVES):
        checksum_path = directory / f"{archive}.sha256"
        records = [
            line
            for line in checksum_path.read_text(encoding="utf-8").splitlines()
            if line.strip()
        ]
        if len(records) != 1:
            raise ContractError(f"{checksum_path.name} must contain exactly one record")
        expected_hash, _ = parse_checksum_line(records[0], archive)
        if sha256(directory / archive) != expected_hash:
            raise ContractError(f"SHA-256 mismatch for {archive}")

    unified: dict[str, str] = {}
    for line in (directory / "sha256.sum").read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        expected_hash, name = parse_checksum_line(line, None)
        if name is None:
            raise ContractError("unified checksum records must name their asset")
        if name in unified:
            raise ContractError(f"duplicate unified checksum record: {name}")
        unified[name] = expected_hash
    if set(unified) != ARCHIVES:
        raise ContractError(
            "unified checksum set mismatch; "
            f"missing={sorted(ARCHIVES - set(unified))!r}, "
            f"unexpected={sorted(set(unified) - ARCHIVES)!r}"
        )
    for name, expected_hash in unified.items():
        if sha256(directory / name) != expected_hash:
            raise ContractError(f"unified SHA-256 mismatch for {name}")


def safe_member(name: str) -> bool:
    trimmed = name[:-1] if name.endswith("/") else name
    raw_parts = trimmed.split("/")
    path = PurePosixPath(name)
    return (
        bool(trimmed)
        and "\\" not in name
        and not any(ord(character) < 0x20 or ord(character) == 0x7F for character in name)
        and not path.is_absolute()
        and all(part not in ("", ".", "..") and ":" not in part for part in raw_parts)
    )


def validate_archive_budget(entries: list[tarfile.TarInfo] | list[zipfile.ZipInfo]) -> None:
    if len(entries) > MAX_ARCHIVE_MEMBERS:
        raise ContractError("archive member count exceeds the validation budget")
    total = sum(
        entry.file_size if isinstance(entry, zipfile.ZipInfo) else entry.size
        for entry in entries
    )
    if total > MAX_ARCHIVE_UNCOMPRESSED_BYTES:
        raise ContractError("archive uncompressed size exceeds the validation budget")


def validate_archive_structure(
    name: str,
    entries: list[tarfile.TarInfo] | list[zipfile.ZipInfo],
    members: list[str],
) -> None:
    validate_archive_budget(entries)
    if not members or any(not safe_member(member) for member in members):
        raise ContractError(f"unsafe or empty archive member set in {name}")
    if len(members) != len(set(members)):
        raise ContractError(f"duplicate archive member in {name}")


def package_identity(manifest: str, label: str) -> tuple[str, str]:
    section = ""
    values: dict[str, str] = {}
    for raw_line in manifest.splitlines():
        line = raw_line.strip()
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1].strip()
            continue
        if section != "package":
            continue
        match = re.fullmatch(r'(name|version)\s*=\s*"([^"\\]+)"\s*(?:#.*)?', line)
        if not match:
            continue
        key, value = match.groups()
        if key in values:
            raise ContractError(f"duplicate package {key} in {label}")
        values[key] = value
    if set(values) != {"name", "version"}:
        raise ContractError(f"missing package name or version in {label}")
    return values["name"], values["version"]


def validate_source_archive(archive: tarfile.TarFile) -> None:
    checkout_name, checkout_version = package_identity(
        ROOT.joinpath("Cargo.toml").read_text(encoding="utf-8"),
        "checkout Cargo.toml",
    )
    prefix = f"{checkout_name}-{checkout_version}"
    expected_files = {f"{prefix}/{path}" for path in SOURCE_FILE_PATHS}
    expected_directories = {prefix}
    for name in expected_files:
        parent = PurePosixPath(name).parent
        while str(parent) != ".":
            expected_directories.add(str(parent))
            parent = parent.parent

    entries = archive.getmembers()
    files = {member.name: member for member in entries if member.isfile()}
    directories = {member.name for member in entries if member.isdir()}
    if set(files) != expected_files or directories != expected_directories:
        raise ContractError(
            "source archive member set mismatch; "
            f"missing_files={sorted(expected_files - set(files))!r}, "
            f"unexpected_files={sorted(set(files) - expected_files)!r}, "
            f"missing_directories={sorted(expected_directories - directories)!r}, "
            f"unexpected_directories={sorted(directories - expected_directories)!r}"
        )
    contents: dict[str, bytes] = {}
    for name in sorted(expected_files):
        member = files[name]
        if member.size == 0:
            raise ContractError(f"source archive member is empty: {name}")
        source = archive.extractfile(member)
        if source is None:
            raise ContractError(f"source archive member cannot be read: {name}")
        contents[name] = source.read()
        relative = PurePosixPath(name).relative_to(prefix)
        checkout_path = ROOT.joinpath(*relative.parts)
        if contents[name] != checkout_path.read_bytes():
            raise ContractError(
                f"source archive member does not match the release checkout: {name}"
            )
        try:
            contents[name].decode("utf-8")
        except UnicodeError as error:
            raise ContractError(f"source archive member is not UTF-8: {name}") from error

    source_identity = package_identity(
        contents[f"{prefix}/Cargo.toml"].decode("utf-8"),
        "source archive Cargo.toml",
    )
    if source_identity != (checkout_name, checkout_version):
        raise ContractError(
            "source archive package identity does not match the release checkout"
        )
    if b"[[package]]" not in contents[f"{prefix}/Cargo.lock"]:
        raise ContractError("source archive Cargo.lock has no package records")


def validate_archives(directory: Path) -> None:
    for name in sorted(ARCHIVES):
        path = directory / name
        try:
            if name.endswith(".zip"):
                with zipfile.ZipFile(path) as archive:
                    entries = archive.infolist()
                    members = [entry.filename for entry in entries]
                    validate_archive_structure(name, entries, members)
                    for entry in entries:
                        mode = entry.external_attr >> 16
                        file_type = stat.S_IFMT(mode)
                        if file_type not in (0, stat.S_IFREG, stat.S_IFDIR):
                            raise ContractError(
                                f"special ZIP member is not allowed in {name}"
                            )
                    bad = archive.testzip()
                    if bad is not None:
                        raise ContractError(f"corrupt ZIP member in {name}: {bad}")
            else:
                with tarfile.open(path, mode="r:*") as archive:
                    entries = archive.getmembers()
                    members = [member.name for member in entries]
                    validate_archive_structure(name, entries, members)
                    if any(not (member.isfile() or member.isdir()) for member in entries):
                        raise ContractError(f"special archive member is not allowed in {name}")
                    if name == "source.tar.gz":
                        validate_source_archive(archive)
        except (tarfile.TarError, zipfile.BadZipFile) as error:
            raise ContractError(f"unreadable archive {name}: {error}") from error

        if name == "source.tar.gz":
            continue
        else:
            executable = "awswit.exe" if name.endswith(".zip") else "awswit"
            if not any(PurePosixPath(member).name == executable for member in members):
                raise ContractError(f"{name} does not contain {executable}")


def validate_json(path: Path) -> None:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (UnicodeError, json.JSONDecodeError) as error:
        raise ContractError(f"invalid JSON asset {path.name}: {error}") from error
    if not isinstance(value, dict) or value.get("dist_version") != "0.31.0":
        raise ContractError(f"unexpected cargo-dist manifest schema in {path.name}")
    package_name, package_version = package_identity(
        ROOT.joinpath("Cargo.toml").read_text(encoding="utf-8"),
        "checkout Cargo.toml",
    )
    if value.get("announcement_tag") != f"v{package_version}":
        raise ContractError(f"unexpected announcement tag in {path.name}")
    releases = value.get("releases")
    if not isinstance(releases, list) or len(releases) != 1:
        raise ContractError(f"unexpected release count in {path.name}")
    release = releases[0]
    if not isinstance(release, dict) or (
        release.get("app_name"), release.get("app_version")
    ) != (package_name, package_version):
        raise ContractError(f"release identity mismatch in {path.name}")


def validate_sbom(path: Path) -> None:
    try:
        root = ElementTree.parse(path).getroot()
    except ElementTree.ParseError as error:
        raise ContractError(f"invalid CycloneDX XML: {error}") from error
    namespace = root.tag.removeprefix("{").partition("}")[0]
    if root.tag.partition("}")[2] != "bom" or not CYCLONEDX_NAMESPACE.fullmatch(
        namespace
    ):
        raise ContractError("CycloneDX SBOM has an unexpected root namespace")
    package_name, package_version = package_identity(
        ROOT.joinpath("Cargo.toml").read_text(encoding="utf-8"),
        "checkout Cargo.toml",
    )
    metadata_component = root.find("./{*}metadata/{*}component")
    if metadata_component is None:
        raise ContractError("CycloneDX SBOM has no metadata component")
    name = metadata_component.findtext("./{*}name")
    version = metadata_component.findtext("./{*}version")
    if (name, version) != (package_name, package_version):
        raise ContractError("CycloneDX SBOM package identity does not match the checkout")
    if not root.findall("./{*}components/{*}component"):
        raise ContractError("CycloneDX SBOM contains no components")


def validate(directory: Path, phase: str) -> None:
    expected = canonical_assets()
    if phase == "build":
        expected |= BUILD_MANIFESTS
    elif phase == "final":
        expected |= FINAL_ADDITIONS
    else:
        raise ContractError(f"unknown validation phase: {phase}")

    validate_flat_exact_set(directory, expected)
    validate_checksums(directory)
    validate_archives(directory)
    if phase == "build":
        for manifest in sorted(BUILD_MANIFESTS):
            validate_json(directory / manifest)
    else:
        validate_json(directory / "dist-manifest.json")
        validate_sbom(directory / "awswit.cdx.xml")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--phase", required=True, choices=("build", "final"))
    parser.add_argument("asset_dir", type=Path)
    args = parser.parse_args()
    try:
        validate(args.asset_dir, args.phase)
    except (ContractError, OSError, tarfile.TarError, zipfile.BadZipFile) as error:
        raise SystemExit(f"release asset contract failed: {error}") from error


if __name__ == "__main__":
    main()
