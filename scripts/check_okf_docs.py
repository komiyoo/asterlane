#!/usr/bin/env python3
"""检查 docs/ 下非保留 Markdown 概念文件的 OKF frontmatter。

保留文件 README.md 和 log.md 不需要 frontmatter。
其余 .md 必须有可解析的 YAML frontmatter 且包含非空 type。
"""

import re
import sys
from pathlib import Path

import yaml

RESERVED = {"README.md", "log.md"}


def main() -> int:
    errors = []
    for path in sorted(Path("docs").rglob("*.md")):
        if path.name in RESERVED:
            continue
        text = path.read_text()
        m = re.match(r"^---\n(.*?)\n---\n", text, re.S)
        if not m:
            errors.append(f"{path}: missing or invalid frontmatter")
            continue
        try:
            data = yaml.safe_load(m.group(1)) or {}
        except yaml.YAMLError as exc:
            errors.append(f"{path}: unparseable frontmatter: {exc}")
            continue
        if not data.get("type"):
            errors.append(f"{path}: missing type")
    if errors:
        print("\n".join(errors))
        return 1
    print("OKF docs check passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
