#!/usr/bin/env python3
"""初始化并检查 Asterlane Git / Cursor Worktree 工作环境。

约定见 docs/engineering/worktree-workflow.md。
本脚本不复制 target/、不复制密钥、不读取 $ROOT_WORKTREE_PATH 下的 .env。
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
from collections.abc import Sequence
from pathlib import Path

MSRV_RE = re.compile(r'^rust-version\s*=\s*"([^"]+)"', re.M)
RUSTC_RE = re.compile(r"rustc\s+(\d+\.\d+(?:\.\d+)?)")
DEFAULT_MSRV = "1.94"


def version_tuple(raw: str) -> tuple[int, ...]:
    parts = []
    for piece in raw.strip().split("."):
        if not piece.isdigit():
            break
        parts.append(int(piece))
    if not parts:
        raise ValueError(f"unparseable version: {raw}")
    return tuple(parts)


def version_ge(actual: str, minimum: str) -> bool:
    return version_tuple(actual) >= version_tuple(minimum)


def parse_msrv(cargo_toml: str) -> str:
    match = MSRV_RE.search(cargo_toml)
    return match.group(1) if match else DEFAULT_MSRV


def find_root(start: Path) -> Path:
    current = start.resolve()
    if current.is_file():
        current = current.parent
    for candidate in (current, *current.parents):
        if (candidate / "Cargo.toml").is_file() and (candidate / "justfile").is_file():
            return candidate
    raise SystemExit(f"asterlane 根目录未找到（从 {start} 向上缺少 Cargo.toml + justfile）")


def is_linked_worktree(root: Path) -> bool | None:
    git_file = root / ".git"
    if git_file.is_file():
        return True
    if git_file.is_dir():
        return False
    return None


def run_capture(args: Sequence[str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, check=False, text=True, capture_output=True)


def rustc_version() -> str | None:
    rustc = shutil.which("rustc")
    if rustc is None:
        return None
    result = run_capture([rustc, "--version"])
    if result.returncode != 0:
        return None
    match = RUSTC_RE.search(result.stdout)
    return match.group(1) if match else None


def has_yaml() -> bool:
    try:
        import yaml  # noqa: F401
    except ImportError:
        return False
    return True


def warn_shared_target(root: Path) -> str | None:
    raw = os.environ.get("CARGO_TARGET_DIR")
    if not raw:
        return None
    target = Path(raw).expanduser()
    if not target.is_absolute():
        target = (Path.cwd() / target).resolve()
    else:
        target = target.resolve()
    try:
        target.relative_to(root.resolve())
    except ValueError:
        return (
            f"CARGO_TARGET_DIR={target} 指向本树之外；并行 Worktree 共用 target 会损坏 incremental，请取消"
        )
    return None


def doctor(root: Path) -> tuple[list[str], list[str], list[str]]:
    errors: list[str] = []
    warnings: list[str] = []
    notes: list[str] = []

    msrv = parse_msrv((root / "Cargo.toml").read_text())
    notes.append(f"root={root}")
    linked = is_linked_worktree(root)
    if linked is True:
        notes.append("git=linked-worktree")
    elif linked is False:
        notes.append("git=primary-checkout")
    else:
        notes.append("git=unknown")

    actual = rustc_version()
    if actual is None:
        errors.append("未找到 rustc（需要 rustup，MSRV 见 Cargo.toml rust-version）")
    elif not version_ge(actual, msrv):
        errors.append(f"rustc {actual} 低于 MSRV {msrv}")
    else:
        notes.append(f"rustc={actual} (>= {msrv})")

    if shutil.which("cargo") is None:
        errors.append("未找到 cargo")
    else:
        notes.append("cargo=ok")

    if shutil.which("python3") is None:
        errors.append("未找到 python3")
    elif not has_yaml():
        errors.append("python3 缺少 PyYAML（文档检查需要：pip install pyyaml）")
    else:
        notes.append("python3+pyyaml=ok")

    if shutil.which("just") is None:
        warnings.append("未找到 just（可选；也可直接跑 cargo fmt/clippy/test）")
    else:
        notes.append("just=ok")

    if shutil.which("rustfmt") is None:
        warnings.append("未找到 rustfmt（init 会尝试 rustup component add rustfmt）")
    else:
        notes.append("rustfmt=ok")

    if shutil.which("cargo-clippy") is None:
        warnings.append("未找到 clippy（init 会尝试 rustup component add clippy）")
    else:
        notes.append("clippy=ok")

    if shutil.which("jq") is None:
        warnings.append("未找到 jq（仅在线 tools CLI 示例需要）")

    shared = warn_shared_target(root)
    if shared:
        warnings.append(shared)
    else:
        notes.append("CARGO_TARGET_DIR=unset-or-in-tree")

    if os.environ.get("ROOT_WORKTREE_PATH"):
        notes.append(
            "ROOT_WORKTREE_PATH 已设置：只作 Cursor 主仓定位，禁止从那里拷 target/ 或 .env"
        )

    example = root / "examples" / "gateway.yaml"
    if not example.is_file():
        errors.append(f"缺少 {example}")
    else:
        notes.append("examples/gateway.yaml=ok")

    return errors, warnings, notes


def ensure_rust_components() -> list[str]:
    warnings: list[str] = []
    rustup = shutil.which("rustup")
    if rustup is None:
        if shutil.which("rustfmt") is None or shutil.which("cargo-clippy") is None:
            warnings.append("未找到 rustup，无法自动安装 rustfmt/clippy")
        return warnings
    result = run_capture([rustup, "component", "add", "rustfmt", "clippy"])
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip() or f"exit {result.returncode}"
        warnings.append(f"rustup component add rustfmt clippy 未成功：{detail}")
    return warnings


def cargo_fetch(root: Path) -> None:
    cargo = shutil.which("cargo")
    if cargo is None:
        raise SystemExit("cargo fetch 失败：未找到 cargo")
    result = subprocess.run(
        [cargo, "fetch", "--manifest-path", str(root / "Cargo.toml")],
        check=False,
    )
    if result.returncode != 0:
        raise SystemExit(f"cargo fetch 失败：exit {result.returncode}")


def print_env(root: Path) -> None:
    config = root / "examples" / "gateway.yaml"
    print("# Asterlane 二进制不自动加载 .env；在本树 shell 中执行下列 export。")
    print(f"export ASTERLANE_CONFIG={config.as_posix()!r}")
    print("# 并行起网关时改端口，并同步 ASTERLANE_SERVER，勿占用主仓 127.0.0.1:3000")
    print("# export ASTERLANE_SERVER='http://127.0.0.1:3100'")
    print("# export ASTERLANE_ADMIN_TOKEN='replace-me-admin-token'")


def emit_report(errors: list[str], warnings: list[str], notes: list[str]) -> None:
    for note in notes:
        print(f"ok    {note}")
    for warning in warnings:
        print(f"warn  {warning}", file=sys.stderr)
    for error in errors:
        print(f"error {error}", file=sys.stderr)


def self_test() -> None:
    assert version_tuple("1.96.0") == (1, 96, 0)
    assert version_ge("1.96.0", "1.94")
    assert version_ge("1.94", "1.94")
    assert not version_ge("1.93.0", "1.94")
    sample = '[package]\nrust-version = "1.94"\n'
    assert parse_msrv(sample) == "1.94"
    assert parse_msrv("[package]\nname = \"x\"\n") == DEFAULT_MSRV
    print("self-test ok")


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        type=Path,
        default=None,
        help="仓库根（默认从当前目录向上查找）",
    )
    parser.add_argument("--doctor", action="store_true", help="只检查，不 cargo fetch")
    parser.add_argument(
        "--print-env",
        action="store_true",
        help="打印本树应 export 的变量（不执行检查）",
    )
    parser.add_argument("--self-test", action="store_true", help="运行脚本内断言")
    return parser.parse_args(argv)


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.self_test:
        self_test()
        return 0

    root = find_root(args.root or Path.cwd())
    if args.print_env:
        print_env(root)
        return 0

    errors, warnings, notes = doctor(root)
    emit_report(errors, warnings, notes)
    if errors:
        return 1
    if args.doctor:
        return 0

    warnings.extend(ensure_rust_components())
    cargo_fetch(root)
    print("init  cargo fetch ok")
    print("next  在本目录运行 just check（或 cargo fmt/clippy/test）")
    print("next  禁止用 ssh mini 'cd ~/wks/aster/asterlane && cargo …' 冒充本树结果")
    return 0


if __name__ == "__main__":
    sys.exit(main())
