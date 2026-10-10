# Contributing

English | [简体中文](CONTRIBUTING.zh-CN.md)

To use the gateway, start with [Running the Gateway](docs/admin/running.md). The problems the project solves are in the [README](README.md); everything else is in the [docs](docs/README.md). The docs are currently written in Chinese. Issues and pull requests in English are welcome.

## Reporting issues

- Bugs: include the version or commit, the expected behavior, the actual behavior, and a minimal config that reproduces it. Use `secret://` references or `replace-me-` placeholders for keys and tokens; never paste real values.
- Features: describe who uses it, what is missing today, and what you expect the gateway to do. Product scope is in [Product Requirements](docs/product/product-requirements.md).
- Vulnerabilities: report them privately through the [security policy](SECURITY.md) only. Do not open a public issue.

Discussions and code review follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Environment

| Dependency | When you need it |
| --- | --- |
| Rust ≥ 1.94 | Building and running the gateway |
| just | Running `just check` |
| Python 3 ≥ 3.10 with `pyyaml` | Docs checks |
| jq | [Running the Gateway](docs/admin/running.md) extracts the token from the issue response |
| Node 22.23.1, Bun 1.4.2, `vp` 1.0.0-rc.0 | Only when changing `web/`. See [web/README.md](web/README.md) for versions |
| cargo-deny | Supply-chain audit when changing dependencies |

## Changing code

1. Branch off `main`. One pull request does one thing.
2. Commit messages use Conventional Commits prefixes: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `ci`, `build`. A scope is allowed, for example `feat(mcp): ...`.
3. If behavior, configuration, error codes, module boundaries, or the admin console change, update the matching concept docs under `docs/`, the category index, and `docs/log.md` in the same PR. The rules are in [Documentation Conventions](docs/engineering/documentation-conventions.md).
4. Add user-visible changes to `## [Unreleased]` in `CHANGELOG.md`. Internal refactors, docs, and CI changes do not need an entry.
5. Run this at the repository root:

```bash
just check
```

Browser regression tests are `just web e2e`, and the deployment rehearsal is `just web deploy-smoke`. Neither is part of `just check`. Initialize a new Git worktree as described in [Worktree Workflow](docs/engineering/worktree-workflow.md) first.

## Review

Use the repository template for the PR description, and fill in the verification table with the actual command results. Maintainers check that layering is preserved, that errors are safe to show to users, and that docs were updated together with the code.

## License

This project is licensed under the [MIT License](LICENSE). By submitting a contribution, you agree to license it under the MIT License. Do not add other license notices in a PR.
