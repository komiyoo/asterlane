#!/usr/bin/env python3
"""检查 docs/ OKF 文档包：frontmatter、分类索引覆盖、子目录导航。

保留文件 README.md、log.md、index.md 不需要 frontmatter。
其余 .md 必须有可解析的 YAML frontmatter 且包含非空 type。
每个概念文件必须能从 docs/README.md 出发、经由保留导航文件中的相对链接到达。
含有概念文件的子目录必须有 README.md。
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

import yaml

RESERVED = {"README.md", "log.md", "index.md"}
NAV_NAMES = {"README.md", "index.md"}
DOCS = Path("docs")
LINK_RE = re.compile(r"\[[^\]]*\]\(([^)]+)\)")


def md_targets(text: str, from_path: Path) -> list[Path]:
    targets = []
    for raw in LINK_RE.findall(text):
        href = raw.strip().split()[0].strip("<>")
        if href.startswith(("http://", "https://", "mailto:", "file:", "#")):
            continue
        href_path = href.split("#", 1)[0]
        if not href_path:
            continue
        dest = (from_path.parent / href_path).resolve()
        if dest.is_dir():
            for nav in ("README.md", "index.md"):
                candidate = dest / nav
                if candidate.is_file():
                    dest = candidate.resolve()
                    break
        targets.append(dest)
    return targets


def collect_concepts() -> dict[Path, Path]:
    found = {}
    for path in sorted(DOCS.rglob("*.md")):
        if path.name in RESERVED:
            continue
        found[path.resolve()] = path
    return found


def check_frontmatter(path: Path) -> list[str]:
    errors: list[str] = []
    text = path.read_text()
    match = re.match(r"^---\n(.*?)\n---\n", text, re.S)
    if not match:
        errors.append(f"{path}: missing or invalid frontmatter")
        return errors
    try:
        data = yaml.safe_load(match.group(1)) or {}
    except yaml.YAMLError as exc:
        errors.append(f"{path}: unparseable frontmatter: {exc}")
        return errors
    if not data.get("type"):
        errors.append(f"{path}: missing type")
    return errors


def walk_index(root_readme: Path) -> tuple[set[Path], list[str]]:
    errors: list[str] = []
    reachable: set[Path] = set()
    seen_nav: set[Path] = set()
    queue = [root_readme.resolve()]
    docs_root = DOCS.resolve()

    while queue:
        current = queue.pop()
        if current in seen_nav:
            continue
        if not current.is_file():
            errors.append(f"{current}: missing navigation file")
            continue
        seen_nav.add(current)
        for dest in md_targets(current.read_text(), current):
            try:
                dest.relative_to(docs_root)
            except ValueError:
                continue
            if dest.name in NAV_NAMES:
                queue.append(dest)
            elif dest.suffix == ".md" and dest.name != "log.md":
                reachable.add(dest)
    return reachable, errors


def check_subdir_indexes(concepts: dict[Path, Path]) -> list[str]:
    errors: list[str] = []
    dirs_with_concepts = {path.parent.resolve() for path in concepts}
    for directory in sorted(dirs_with_concepts):
        if directory == DOCS.resolve():
            continue
        if not (directory / "README.md").is_file():
            rel = directory.relative_to(Path.cwd())
            errors.append(f"{rel}: concept subdirectory missing README.md")
    return errors


def main() -> int:
    if not DOCS.is_dir():
        print("docs/: directory not found")
        return 1

    errors: list[str] = []
    concepts = collect_concepts()
    for rel in concepts.values():
        errors.extend(check_frontmatter(rel))
    errors.extend(check_subdir_indexes(concepts))

    root_readme = DOCS / "README.md"
    if not root_readme.is_file():
        errors.append("docs/README.md: missing")
    else:
        reachable, walk_errors = walk_index(root_readme)
        errors.extend(walk_errors)
        for resolved, rel in concepts.items():
            if resolved not in reachable:
                errors.append(f"{rel}: not linked from docs/README.md index tree")

    if errors:
        print("\n".join(errors))
        return 1
    print("OKF docs check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
