# LabWired Release & Merging Strategy

## 1. Branching Model: Trunk-Based Development
We use **trunk-based development** on the `main` branch with short-lived feature branches.
- **`main`**: The primary development branch. All work happens here via feature branches and PRs. Tags are created here for releases.
- **`feature/*`**: Short-lived branches for individual work items. Created from `main`, merged back to `main` via PR.
    - Naming convention: `feature/short-description` or `feature/issue-id-description`.
- **`hotfix/*`**: Critical bug fixes. Created from `main`, merged back to `main` via PR.

### Merging Rules
- **Pull Requests (PRs)** are mandatory for all merges.
- **Approvals**: At least 1 review approval is required.
- **CI Checks**: All checks (Build, Test, Lint, Audit) must pass.
- **History**: Use "Squash and Merge" for feature branches to keep history clean. Use "Merge Commit" for releases to preserve the valid history.

## 2. Quality Gates
Every PR and commit to `main` must pass the following automated gates. **Developers MUST run these locally before opening a PR.**

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

### 3.2. Release Notes Format (Verbatim)
When preparing a release, the following verbatim format MUST be used for the GitHub Release notes:

**Title**: `Release vX.Y.Z: <Short Summary of Key Changes>`

**Body Structure**:
```markdown
Features:
- <Feature 1 description>
- <Feature 2 description>

Improvements:
- <Improvement 1 description>
- <Improvement 2 description>

Fixes: (Optional)
- <Fix 1 description>
```

*Example:*
> **Release v0.9.0: Variable PD Types and Integration Testing**
>
> Features:
> - Variable PD types (1_V and 2_V) with dynamic length changes (2-32 bytes)
> - Integration testing infrastructure (test_device_connection.py, test_integration.sh)
>
> Improvements:
> - Enhanced Virtual Master with set_pd_length() method
> - Comprehensive test suites for all M-sequence types

### 3.3. Steps to Release
1.  **Prepare**: Create a release preparation branch from `main` (optional, can also work directly on `main`).
2.  **Bump**: Update version numbers in `Cargo.toml` (workspace and crates).
3.  **Changelog**: Update `CHANGELOG.md` with features and fixes.
4.  **Verify**: Run the full regression suite, lints, and formatting check locally:
    - Host: `cargo test --workspace --exclude firmware`
    - Firmware: `cargo build -p firmware --target thumbv7m-none-eabi`
    - Lints: `cargo clippy --workspace --exclude firmware -- -D warnings`
    - Format: `cargo fmt --all -- --check`
5.  **Draft Release**: Create a GitHub Release draft using the **Verbatim** format above.
6.  **Merge & Tag**:
    - Merge changes to `main` via PR.
    - Tag `main` with `vX.Y.Z`.
7.  **Publish**:
    -   Push the `vX.Y.Z` tag to trigger release workflows.
    -   Ensure CI validation workflows are green (`.github/workflows/core-ci.yml` and `.github/workflows/vscode-ci.yml`).
    -   Publish release artifacts (CLI binaries, docs, and extension package) using the current manual publishing process.
    -   If/when a dedicated release workflow is added, document it here and make it part of the required gates.

## 4. Coding Standards documentation
- **Style**: Follow standard Rust style (`rustfmt`).
- **Docs**: Public APIs must be documented (`/// doc comments`).
- **Errors**: Use `thiserror` for library errors and `anyhow` for application/CLI errors.
- **Commits**: Follow Conventional Commits (e.g., `feat: allow loading hex files`, `fix: resolve crash on empty input`).
