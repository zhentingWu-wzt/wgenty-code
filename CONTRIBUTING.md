# Contributing to Wgenty Code

Thanks for your interest in contributing! This document outlines the conventions and workflow.

## Development Setup

```bash
git clone https://github.com/zhentingWu-wzt/wgenty-code.git
cd wgenty-code
cargo build
```

## Code Style

- Follow [Rust standard naming conventions](https://rust-lang.github.io/api-guidelines/naming.html)
  - `snake_case` for variables/functions, `CamelCase` for types/traits, `SCREAMING_SNAKE_CASE` for constants
- Run `cargo fmt` before committing (CI enforces `cargo fmt -- --check`)
- Zero clippy warnings: `cargo clippy --all-targets -- -D warnings`

## Commit Convention

Follow [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <short description>

<body>

<footer>
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `chore`

Examples:
```
feat(cli): add config reset command
fix(sandbox): handle missing kernel support gracefully
docs(readme): update installation instructions
```

## Branch & PR Workflow

1. Create a feature branch from `develop`: `feature/<description>`, `fix/<description>`, `refactor/<description>`
2. Make changes, commit following Conventional Commits
3. Open a PR to `develop` with a Conventional Commits title
4. Ensure CI passes (check, test, fmt, clippy)

### PR Checklist

- [ ] `cargo clippy --all-targets -- -D warnings` — zero warnings
- [ ] `cargo fmt` — consistent formatting
- [ ] `cargo test --all` — all tests pass
- [ ] Complex changes have explanatory comments
- [ ] Update CHANGELOG.md

## Performance Constraints

New code must not significantly regress:

- **Startup time**: increment ≤ 5%
- **Memory usage**: ≤ 2% increase
- **Binary size**: ≤ 500 KB increase

Verify with:
```bash
cargo build --release
time ./target/release/wgenty-code --version
ls -lh ./target/release/wgenty-code
```

## Module Dependency Rules

- `tools/` must not depend on `agent/`
- `api/` must not depend on `cli/` or `tui/`
- `config/` must not depend on any business module
- Cross-layer dependencies use trait abstractions (e.g., `SandboxBackend`, `Tool`)

## Error Handling

- Library code: use `thiserror` for custom error enums
- Application code: use `anyhow::Result` + `.context("description")`
- Never bare `unwrap()` or `?` without context

## Security Considerations

- Changes to `guardian/`, `sandbox/`, or `permissions/` require extra scrutiny
- `is_read_only()` defaults to `false` — read-only tools must explicitly return `true`
- Critical-risk operations are auto-denied by guardian

## Questions?

Open a [GitHub Issue](https://github.com/zhentingWu-wzt/wgenty-code/issues) or start a [Discussion](https://github.com/zhentingWu-wzt/wgenty-code/discussions).
