//! 把管理 HTTP 契约写成 `schemas/admin.json`。
//!
//! 检查模式只比较，不覆盖。失败时提示 `just admin-schema`。

use anyhow::{Context, Result, bail};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> Result<ExitCode> {
    let mut check = false;
    let mut output = PathBuf::from("schemas/admin.json");
    let mut args = env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check" => {
                check = true;
                if args.peek().is_some_and(|next| !next.starts_with('-')) {
                    output = PathBuf::from(args.next().context("check path")?);
                }
            }
            "--output" => {
                output = PathBuf::from(args.next().context("--output requires a path")?);
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    let rendered =
        asterlane::admin::schema::render_admin_schema().context("render admin schema")?;
    if check {
        let existing =
            fs::read_to_string(&output).with_context(|| format!("read {}", output.display()))?;
        if existing != rendered {
            eprintln!(
                "{} is out of date with the Rust admin DTOs.\nRegenerate with: just admin-schema",
                output.display()
            );
            return Ok(ExitCode::from(1));
        }
        return Ok(ExitCode::SUCCESS);
    }

    if let Some(parent) = output.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;
    }
    fs::write(&output, rendered).with_context(|| format!("write {}", output.display()))?;
    Ok(ExitCode::SUCCESS)
}
