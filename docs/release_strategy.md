# LabWired Release & Merging Strategy

This document defines release policy and branch governance.
Release execution authority is `core/RELEASE_PROCESS.md`.

## 1. Branching Model: Gitflow
We follow the **Gitflow** branching strategy to manage releases and features efficiently. For more detailed rules and workflows, see the [Git Flow Guide](./development/git_flow.md).
- **`main`**: The production-ready state. Only merge from `release/*` or `hotfix/*`. Tags are created here.
- **`develop`**: The integration branch for the next release. Features merge here.
- **`feature/*`**: Individual work items. Created from `develop`, merged back to `develop`.
    - Naming convention: `feature/short-description` or `feature/issue-id-description`.
- **`release/*`**: Preparation for a new production release. Created from `develop`, merged to `main` AND `develop`.
- **`hotfix/*`**: Critical bug fixes for production. Created from `main`, merged to `main` AND `develop`.

### Merging Rules
- **Pull Requests (PRs)** are mandatory for all merges.
- **Approvals**: At least 1 review approval is required.
- **CI Checks**: All checks (Build, Test, Lint, Audit) must pass.
- **History**: Use "Squash and Merge" for feature branches to keep history clean. Use "Merge Commit" for releases to preserve the valid history.

## 2. Quality Gates
Every PR and commit to `develop`/`main` must pass automated gates. Developers should run matching checks locally before opening a PR.

### Automated Checks (CI)
| Check | Command | Failure Condition |
| :--- | :--- | :--- |
| **Formatting** | `cargo fmt -- --check` | Any formatting violation. |
| **Linting** | `cargo clippy -- -D warnings` | Any warnings or errors. |
| **Tests** | `cargo test` | Any test failure. |
| **Security** | `cargo audit` | Known vulnerabilities in dependencies. |
| **Build** | `cargo build` | Compilation error. |

### Test Coverage
- **Goal**: >80% Code Coverage.
- **Tool**: `cargo-tarpaulin`.
- **Enforcement**: CI will generate a coverage report. Significant drops in coverage should block the PR.

## 3. Release Policy

### 3.1. Release Notes Format
Release notes must use the shared template at:

- `.github/RELEASE_TEMPLATE.md`

Do not publish a release with missing template sections (scope, validation evidence, CI links, known issues, breaking changes).

### 3.2. Steps to Release
Use `core/RELEASE_PROCESS.md` as the execution runbook for monorepo release gates.

Minimum mandatory gates include:

1. Full workspace validation (`cargo test --workspace`, `cargo build --workspace`).
2. Unsupported instruction audit with fail-on-unsupported.
3. Cross-component compatibility baseline checks, even for scoped releases.
4. Runtime smoke tests for CI fixtures and impacted board examples.
5. Green required CI workflows before tagging.
6. GitHub release draft created from `.github/RELEASE_TEMPLATE.md`.

## 4. Coding Standards documentation
- **Style**: Follow standard Rust style (`rustfmt`).
- **Docs**: Public APIs must be documented (`/// doc comments`).
- **Errors**: Use `thiserror` for library errors and `anyhow` for application/CLI errors.
- **Commits**: Follow Conventional Commits (e.g., `feat: allow loading hex files`, `fix: resolve crash on empty input`).
