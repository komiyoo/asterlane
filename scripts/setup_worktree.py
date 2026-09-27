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
VP_VERSION_RE = re.compile(r"(\d+\.\d+\.\d+(?:-[\w.]+)?)")
DEFAULT_MSRV = "1.94"
REQUIRED_VP_VERSION = "1.0.0-rc.0"


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


def parse_vp_version(text: str) -> str | None:
    match = VP_VERSION_RE.search(text)
    return match.group(1) if match else None


def vp_version() -> str | None:
    vp = shutil.which("vp")
    if vp is None:
        return None
    result = run_capture([vp, "--version"])
    if result.returncode != 0:
        return None
    return parse_vp_version(result.stdout)


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

    actual_vp = vp_version()
    node_modules = root / "web" / "node_modules"
    if actual_vp is None:
        warnings.append(
            f"未找到 vp {REQUIRED_VP_VERSION}。纯 Cargo 构建不需要它；前端检查需要本机安装这一版"
        )
    elif actual_vp != REQUIRED_VP_VERSION:
        warnings.append(f"vp {actual_vp} 与仓库要求的 {REQUIRED_VP_VERSION} 不一致，不要改工具链")
    else:
        notes.append(f"vp={actual_vp}")
    if node_modules.is_symlink():
        warnings.append("web/node_modules 是符号链接；每棵树应各自 vp install --frozen-lockfile")
    elif not node_modules.exists():
        warnings.append("未安装 web/node_modules（有 vp 时 just worktree init 会冻结安装）")
    else:
        notes.append("web/node_modules=present")

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


PROTECTED_BRANCHES = frozenset({"main", "master"})
WORKTREE_DIR_NAME = ".worktrees"


def git(root: Path, *args: str) -> subprocess.CompletedProcess[str]:
    return run_capture(["git", "-C", str(root), *args])


def parse_worktree_porcelain(text: str) -> list[dict[str, str]]:
    rows: list[dict[str, str]] = []
    current: dict[str, str] = {}
    for raw in text.splitlines():
        line = raw.rstrip()
        if not line:
            if current:
                rows.append(current)
                current = {}
            continue
        key, _, value = line.partition(" ")
        if key == "worktree":
            if current:
                rows.append(current)
            current = {"path": value}
        elif key == "branch":
            current["branch"] = value.removeprefix("refs/heads/")
        elif key == "HEAD":
            current["head"] = value
        elif key == "detached":
            current["detached"] = "1"
    if current:
        rows.append(current)
    return rows


def list_worktrees(root: Path) -> list[dict[str, str]]:
    result = git(root, "worktree", "list", "--porcelain")
    if result.returncode != 0:
        detail = (result.stderr or result.stdout).strip() or f"exit {result.returncode}"
        raise SystemExit(f"git worktree list 失败：{detail}")
    return parse_worktree_porcelain(result.stdout)


def merged_local_branches(root: Path, into: str = "main") -> list[str]:
    result = git(root, "branch", "--merged", into)
    if result.returncode != 0:
        return []
    names = []
    for raw in result.stdout.splitlines():
        name = raw.replace("*", "").strip()
        if name:
            names.append(name)
    return names


def current_branch(root: Path) -> str | None:
    result = git(root, "branch", "--show-current")
    if result.returncode != 0:
        return None
    name = result.stdout.strip()
    return name or None


def prune_remnants(root: Path, *, delete_merged: bool) -> int:
    if is_linked_worktree(root) is True:
        raise SystemExit("请在主 checkout 运行 prune，不要在功能树里清理主仓残留")

    prune = git(root, "worktree", "prune", "-v")
    if prune.returncode != 0:
        detail = (prune.stderr or prune.stdout).strip() or f"exit {prune.returncode}"
        raise SystemExit(f"git worktree prune 失败：{detail}")
    if prune.stdout.strip():
        print(prune.stdout, end="" if prune.stdout.endswith("\n") else "\n")
    else:
        print("ok    git worktree prune（无失效登记）")

    extras = [
        row
        for row in list_worktrees(root)
        if Path(row["path"]).resolve() != root.resolve()
    ]
    attached_branches = {row["branch"] for row in extras if row.get("branch")}
    for row in extras:
        label = row.get("branch") or ("detached" if row.get("detached") else row.get("head", "?"))
        print(f"keep  {row['path']} [{label}]  # 先合进 main，再 git worktree remove")

    leftover_dir = root / WORKTREE_DIR_NAME
    if leftover_dir.is_dir() and not any(leftover_dir.iterdir()):
        leftover_dir.rmdir()
        print(f"rm    空目录 {leftover_dir}")
    elif leftover_dir.is_dir():
        print(f"keep  {leftover_dir}（非空，需先 remove 其中的树）")

    head = current_branch(root)
    removable = [
        name
        for name in merged_local_branches(root)
        if name not in PROTECTED_BRANCHES
        and name != head
        and name not in attached_branches
    ]
    for name in removable:
        if delete_merged:
            deleted = git(root, "branch", "-d", name)
            if deleted.returncode != 0:
                detail = (deleted.stderr or deleted.stdout).strip()
                print(f"warn  无法删除已合并分支 {name}：{detail}", file=sys.stderr)
            else:
                print(f"rm    已合并分支 {name}")
        else:
            print(f"hint  已合并分支 {name}  # just worktree prune --merged")

    print("next  不要删主仓 target/ 或 rustup；功能树目录用 git worktree remove / Cursor /delete-worktree")
    return 0


def install_web_deps(root: Path) -> None:
    web = root / "web"
    lock = web / "bun.lock"
    if not lock.is_file():
        raise SystemExit(f"缺少 {lock}，无法冻结安装前端依赖")
    node_modules = web / "node_modules"
    if node_modules.is_symlink():
        raise SystemExit("web/node_modules 是符号链接。删除它后重新 init，不要把依赖指到别的树")
    version = vp_version()
    if version is None:
        print(
            f"warn  未找到 vp {REQUIRED_VP_VERSION}，跳过 web/ 依赖安装。cargo 构建不需要它",
            file=sys.stderr,
        )
        return
    if version != REQUIRED_VP_VERSION:
        raise SystemExit(f"vp {version} 与仓库要求的 {REQUIRED_VP_VERSION} 不一致，不要升级或降级")
    vp = shutil.which("vp")
    if vp is None:
        raise SystemExit(f"未找到 vp {REQUIRED_VP_VERSION}")
    result = subprocess.run([vp, "install", "--frozen-lockfile"], cwd=web, check=False)
    if result.returncode != 0:
        raise SystemExit(f"vp install --frozen-lockfile 失败：exit {result.returncode}")
    if node_modules.is_symlink() or not node_modules.is_dir():
        raise SystemExit("web/node_modules 没有装在本树目录里")
    print("init  web dependencies ok")


def print_env(root: Path) -> None:
    config = root / "examples" / "gateway.yaml"
    print("# Asterlane 二进制不自动加载 .env；在本树 shell 中执行下列 export。")
    print(f"export ASTERLANE_CONFIG={config.as_posix()!r}")
    print("# 并行起网关时改端口，并同步 ASTERLANE_SERVER，勿占用主仓 127.0.0.1:3000")
    print("# export ASTERLANE_SERVER='http://127.0.0.1:3100'")
    print("# export ASTERLANE_DEV_GATEWAY_PORT=3100  # 只填端口，vp dev 把 /admin 代理到 127.0.0.1")
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
    assert parse_vp_version("vp v1.0.0-rc.0\n") == "1.0.0-rc.0"
    assert parse_vp_version("no version") is None
    rows = parse_worktree_porcelain(
        "worktree /tmp/main\nHEAD abc\nbranch refs/heads/main\n\n"
        "worktree /tmp/feat\nHEAD def\nbranch refs/heads/feat/x\n"
    )
    assert [row["path"] for row in rows] == ["/tmp/main", "/tmp/feat"]
    assert rows[1]["branch"] == "feat/x"
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
    parser.add_argument(
        "--prune",
        action="store_true",
        help="在主 checkout 清理失效 worktree 登记和空的 .worktrees/",
    )
    parser.add_argument(
        "--delete-merged-branches",
        action="store_true",
        help="与 --prune 一起：删除已合进 main 且无 worktree 占用的本地分支",
    )
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
    if args.prune:
        return prune_remnants(root, delete_merged=args.delete_merged_branches)
    if args.delete_merged_branches:
        raise SystemExit("--delete-merged-branches 必须与 --prune 一起使用")

    errors, warnings, notes = doctor(root)
    emit_report(errors, warnings, notes)
    if errors:
        return 1
    if args.doctor:
        return 0

    warnings.extend(ensure_rust_components())
    cargo_fetch(root)
    print("init  cargo fetch ok")
    install_web_deps(root)
    print("next  在本目录运行 just check（含前端静态检查、测试和构建；端到端另跑 just web e2e）")
    print("next  做完合回 main 后，在主仓运行 just worktree prune")
    return 0


if __name__ == "__main__":
    sys.exit(main())
