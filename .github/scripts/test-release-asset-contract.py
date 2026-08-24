#!/usr/bin/env python3
"""Deterministic positive and negative fixtures for release asset validation."""

from __future__ import annotations

import importlib.util
import io
import json
import tarfile
import tempfile
import unittest
import zipfile
from pathlib import Path


SCRIPT = Path(__file__).with_name("validate-release-assets.py")
SPEC = importlib.util.spec_from_file_location("release_asset_contract", SCRIPT)
if SPEC is None or SPEC.loader is None:
    raise RuntimeError("could not load release asset validator")
contract = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(contract)


def add_tar_file(archive: tarfile.TarFile, name: str, contents: bytes) -> None:
    member = tarfile.TarInfo(name)
    member.size = len(contents)
    archive.addfile(member, io.BytesIO(contents))


def add_tar_directory(archive: tarfile.TarFile, name: str) -> None:
    member = tarfile.TarInfo(name)
    member.type = tarfile.DIRTYPE
    archive.addfile(member)


def refresh_checksums(directory: Path) -> None:
    unified = []
    for archive_name in sorted(contract.ARCHIVES):
        digest = contract.sha256(directory / archive_name)
        directory.joinpath(f"{archive_name}.sha256").write_text(
            f"{digest}  {archive_name}\n", encoding="utf-8"
        )
        unified.append(f"{digest}  {archive_name}")
    directory.joinpath("sha256.sum").write_text(
        "\n".join(unified) + "\n", encoding="utf-8"
    )


def write_source_archive(
    path: Path,
    *,
    cargo_toml: bytes | None = None,
    omitted: frozenset[str] = frozenset(),
    extra: dict[str, bytes] | None = None,
) -> None:
    package_name, package_version = contract.package_identity(
        contract.ROOT.joinpath("Cargo.toml").read_text(encoding="utf-8"),
        "fixture checkout Cargo.toml",
    )
    prefix = f"{package_name}-{package_version}"
    included = contract.SOURCE_FILE_PATHS - omitted
    with tarfile.open(path, mode="w:gz") as archive:
        directories = {prefix}
        for relative in included:
            parent = Path(prefix, relative).parent
            while str(parent) != ".":
                directories.add(parent.as_posix())
                parent = parent.parent
        for directory in sorted(directories, key=lambda value: (value.count("/"), value)):
            add_tar_directory(archive, directory)
        for relative in sorted(included):
            contents = (
                cargo_toml
                if relative == "Cargo.toml" and cargo_toml is not None
                else contract.ROOT.joinpath(relative).read_bytes()
            )
            add_tar_file(archive, f"{prefix}/{relative}", contents)
        for relative, contents in sorted((extra or {}).items()):
            add_tar_file(archive, f"{prefix}/{relative}", contents)


def create_fixture(directory: Path, phase: str) -> None:
    for installer in ("awswit-installer.sh", "awswit-installer.ps1"):
        directory.joinpath(installer).write_text(f"fixture {installer}\n", encoding="utf-8")

    for archive_name in sorted(contract.ARCHIVES):
        path = directory / archive_name
        if archive_name.endswith(".zip"):
            with zipfile.ZipFile(path, mode="w") as archive:
                archive.writestr("awswit-x86_64-pc-windows-msvc/awswit.exe", b"binary")
        elif archive_name == "source.tar.gz":
            write_source_archive(path)
        else:
            target = archive_name.removeprefix("awswit-").removesuffix(".tar.xz")
            with tarfile.open(path, mode="w:xz") as archive:
                add_tar_file(archive, f"awswit-{target}/awswit", b"binary")

    refresh_checksums(directory)

    package_name, package_version = contract.package_identity(
        contract.ROOT.joinpath("Cargo.toml").read_text(encoding="utf-8"),
        "fixture checkout Cargo.toml",
    )
    manifest = json.dumps(
        {
            "dist_version": "0.31.0",
            "announcement_tag": f"v{package_version}",
            "releases": [
                {"app_name": package_name, "app_version": package_version}
            ],
        }
    ) + "\n"
    if phase == "build":
        for name in contract.BUILD_MANIFESTS:
            directory.joinpath(name).write_text(manifest, encoding="utf-8")
    elif phase == "final":
        directory.joinpath("dist-manifest.json").write_text(manifest, encoding="utf-8")
        directory.joinpath("awswit.cdx.xml").write_text(
            '<bom xmlns="http://cyclonedx.org/schema/bom/1.6" version="1">'
            '<metadata><component type="application">'
            f'<name>{package_name}</name><version>{package_version}</version>'
            '</component></metadata><components>'
            '<component type="library"><name>dependency</name></component>'
            "</components></bom>\n",
            encoding="utf-8",
        )
    else:
        raise ValueError(phase)


class ReleaseAssetContractTests(unittest.TestCase):
    def fixture(self, phase: str) -> tuple[tempfile.TemporaryDirectory[str], Path]:
        temporary = tempfile.TemporaryDirectory()
        directory = Path(temporary.name)
        create_fixture(directory, phase)
        return temporary, directory

    def assert_rejected(self, phase: str, mutate) -> None:
        temporary, directory = self.fixture(phase)
        with temporary:
            mutate(directory)
            with self.assertRaises(contract.ContractError):
                contract.validate(directory, phase)

    def test_complete_build_and_final_sets_are_accepted(self) -> None:
        for phase in ("build", "final"):
            temporary, directory = self.fixture(phase)
            with temporary:
                contract.validate(directory, phase)

    def test_missing_installer_is_rejected(self) -> None:
        self.assert_rejected(
            "build", lambda directory: directory.joinpath("awswit-installer.sh").unlink()
        )

    def test_unexpected_asset_is_rejected(self) -> None:
        self.assert_rejected(
            "build", lambda directory: directory.joinpath("unexpected.bin").write_bytes(b"x")
        )

    def test_checksum_mismatch_is_rejected(self) -> None:
        self.assert_rejected(
            "build",
            lambda directory: directory.joinpath(
                "awswit-x86_64-unknown-linux-gnu.tar.xz"
            ).write_bytes(b"tampered"),
        )

    def test_incomplete_unified_checksum_set_is_rejected(self) -> None:
        def remove_record(directory: Path) -> None:
            checksum = directory / "sha256.sum"
            lines = checksum.read_text(encoding="utf-8").splitlines()
            checksum.write_text("\n".join(lines[1:]) + "\n", encoding="utf-8")

        self.assert_rejected("build", remove_record)

    def test_checksum_valid_but_unreadable_source_archive_is_rejected(self) -> None:
        def corrupt_source(directory: Path) -> None:
            directory.joinpath("source.tar.gz").write_bytes(b"not a tar archive")
            refresh_checksums(directory)

        self.assert_rejected("build", corrupt_source)

    def test_checksum_valid_but_traversing_archive_is_rejected(self) -> None:
        def replace_archive(directory: Path) -> None:
            path = directory / "awswit-x86_64-unknown-linux-gnu.tar.xz"
            with tarfile.open(path, mode="w:xz") as archive:
                add_tar_file(archive, "../awswit", b"binary")
            refresh_checksums(directory)

        self.assert_rejected("build", replace_archive)

    def test_archive_structure_rejects_control_paths_and_duplicates(self) -> None:
        entries = [tarfile.TarInfo("root/awswit"), tarfile.TarInfo("root/awswit")]
        with self.assertRaises(contract.ContractError):
            contract.validate_archive_structure(
                "fixture.tar.xz", entries, [entry.name for entry in entries]
            )

        controlled = tarfile.TarInfo("root/forged\nmember")
        with self.assertRaises(contract.ContractError):
            contract.validate_archive_structure(
                "fixture.tar.xz", [controlled], [controlled.name]
            )

    def test_archive_budget_preflights_member_count_and_uncompressed_size(self) -> None:
        entries = [tarfile.TarInfo(f"root/{index}") for index in range(
            contract.MAX_ARCHIVE_MEMBERS + 1
        )]
        with self.assertRaises(contract.ContractError):
            contract.validate_archive_budget(entries)

        oversized = tarfile.TarInfo("root/oversized")
        oversized.size = contract.MAX_ARCHIVE_UNCOMPRESSED_BYTES + 1
        with self.assertRaises(contract.ContractError):
            contract.validate_archive_budget([oversized])

    def test_source_package_identity_mismatch_is_rejected(self) -> None:
        def replace_source(directory: Path) -> None:
            package_name, _package_version = contract.package_identity(
                contract.ROOT.joinpath("Cargo.toml").read_text(encoding="utf-8"),
                "fixture checkout Cargo.toml",
            )
            write_source_archive(
                directory / "source.tar.gz",
                cargo_toml=(
                    f'[package]\nname = "{package_name}"\nversion = "9.9.9"\n'
                ).encode(),
            )
            refresh_checksums(directory)

        self.assert_rejected("build", replace_source)

    def test_source_archive_missing_core_source_is_rejected(self) -> None:
        def remove_core_source(directory: Path) -> None:
            write_source_archive(
                directory / "source.tar.gz",
                omitted=frozenset({"src/lib.rs"}),
            )
            refresh_checksums(directory)

        self.assert_rejected("build", remove_core_source)

    def test_source_archive_with_unexpected_member_is_rejected(self) -> None:
        def add_unexpected_source(directory: Path) -> None:
            write_source_archive(
                directory / "source.tar.gz",
                extra={"unreviewed-source.rs": b"fn unreviewed() {}\n"},
            )
            refresh_checksums(directory)

        self.assert_rejected("build", add_unexpected_source)

    def test_malformed_sbom_is_rejected(self) -> None:
        self.assert_rejected(
            "final",
            lambda directory: directory.joinpath("awswit.cdx.xml").write_text(
                "<not-xml", encoding="utf-8"
            ),
        )

    def test_sbom_identity_mismatch_is_rejected(self) -> None:
        self.assert_rejected(
            "final",
            lambda directory: directory.joinpath("awswit.cdx.xml").write_text(
                '<bom xmlns="http://cyclonedx.org/schema/bom/1.6" version="1">'
                '<metadata><component type="application">'
                '<name>other</name><version>9.9.9</version>'
                '</component></metadata><components>'
                '<component type="library"><name>dependency</name></component>'
                '</components></bom>\n',
                encoding="utf-8",
            ),
        )


if __name__ == "__main__":
    unittest.main()
