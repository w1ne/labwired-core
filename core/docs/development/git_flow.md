# LabWired Git Flow

This document details our Git Flow branching strategy and the rules governing branch management.

## 1. Branch Hierarchy

### Permanent Branches
- **`main`**: Represents the latest stable, production-ready release.
- **`develop`**: The integration branch for the next release. Contains the latest successfully merged features.

### Supporting Branches
- **`feature/*`**: For new features, enhancements, or experiments.
    - Base: `develop`
    - Target: `develop` via Pull Request.
- **`release/*`**: For preparing a new production release.
    - Base: `develop`
    - Target: `main` AND `develop` via Pull Request.
- **`hotfix/*`**: For critical bug fixes in production.
    - Base: `main`
    - Target: `main` AND `develop` via Pull Request.

## 2. Working Workflow

1.  **Create a branch**: Start from `develop` for features or `main` for hotfixes.
2.  **Commit changes**: Follow [Conventional Commits](https://www.conventionalcommits.org/).
3.  **Open a Pull Request**: Target the appropriate branch (`develop` or `main`).
4.  **Wait for CI**: All automated tests, lints, and audits MUST pass.
5.  **Resolve Conversations**: All comments must be addressed or resolved.
6.  **Merge**:
    - `feature/*` -> `develop`: Squash and Merge.
    - `release/*` -> `main`: Merge Commit (to preserve versioning history).
    - `hotfix/*` -> `main`: Merge Commit.

## 3. Branch Protection Rules

The `main` and `develop` branches are protected with the following rules:

- **Require status checks to pass**: The CI `build` job (including tests, lints, and audits) must succeed.
- **Require conversation resolution**: All discussions must be closed.

## 4. Feature Implementation Rule
New functionality can ONLY be merged after a Pull Request is approved and all CI tests are confirmed green.
