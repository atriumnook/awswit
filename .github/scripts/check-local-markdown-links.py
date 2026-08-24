#!/usr/bin/env python3
"""Fail when a repository-local Markdown link or heading target is missing."""

from __future__ import annotations

import re
import sys
from pathlib import Path
from urllib.parse import unquote, urlsplit


ROOT = Path(__file__).resolve().parents[2]
LINK = re.compile(
    r"!?\[[^\]]*\]\((<[^>]+>|[^\s)]+)(?:\s+(?:\"[^\"]*\"|'[^']*'))?\)"
)
HEADING = re.compile(r"^#{1,6}\s+(.+?)\s*#*\s*$")
EXPLICIT_ANCHOR = re.compile(r"<(?:a\s+name|[^>]+\sid)=[\"']([^\"']+)[\"']", re.I)
INLINE_LINK = re.compile(r"!?\[([^\]]*)\]\([^)]*\)")
HTML_TAG = re.compile(r"<[^>]+>")


def github_slug(heading: str) -> str:
    """Approximate GitHub's documented heading slug for repository headings."""

    text = INLINE_LINK.sub(r"\1", heading)
    text = HTML_TAG.sub("", text)
    text = text.replace("`", "").lower().strip()
    text = "".join(character for character in text if character.isalnum() or character in " _-")
    return re.sub(r"\s", "-", text)


def anchors(path: Path) -> set[str]:
    found: set[str] = set()
    occurrences: dict[str, int] = {}
    for line in path.read_text(encoding="utf-8").splitlines():
        match = HEADING.match(line)
        if match:
            base = github_slug(match.group(1))
            suffix = occurrences.get(base, 0)
            occurrences[base] = suffix + 1
            found.add(base if suffix == 0 else f"{base}-{suffix}")
        found.update(EXPLICIT_ANCHOR.findall(line))
    return found


def main() -> int:
    failures: list[str] = []
    anchor_cache: dict[Path, set[str]] = {}
    for source in sorted(ROOT.rglob("*.md")):
        if ".git" in source.parts or "target" in source.parts:
            continue
        for line_number, line in enumerate(source.read_text(encoding="utf-8").splitlines(), 1):
            for match in LINK.finditer(line):
                raw = match.group(1).strip("<>")
                parsed = urlsplit(raw)
                if parsed.scheme or parsed.netloc:
                    continue
                relative = unquote(parsed.path)
                target = source if not relative else (ROOT / relative.lstrip("/")) if relative.startswith("/") else source.parent / relative
                target = target.resolve()
                try:
                    target.relative_to(ROOT)
                except ValueError:
                    failures.append(f"{source.relative_to(ROOT)}:{line_number}: link escapes repository: {raw}")
                    continue
                if not target.is_file():
                    failures.append(f"{source.relative_to(ROOT)}:{line_number}: missing file: {raw}")
                    continue
                if parsed.fragment and target.suffix.lower() == ".md":
                    available = anchor_cache.setdefault(target, anchors(target))
                    fragment = unquote(parsed.fragment).lower()
                    if fragment not in available:
                        failures.append(
                            f"{source.relative_to(ROOT)}:{line_number}: missing heading #{fragment} in {target.relative_to(ROOT)}"
                        )
    if failures:
        print("\n".join(failures), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
