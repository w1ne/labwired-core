# Contributing to LabWired

Thank you for your interest in contributing to LabWired! We welcome contributions from the community.

## Development Workflow

We follow a **Trunk-Based Development** workflow:
- **`main`**: The primary branch for active development.
- **`feature/name`**: Your working branch. Open PRs against `main`.

### 1. Setup
```bash
git clone https://github.com/w1ne/labwired.git
cd labwired
cargo build
```

### 2. Making Changes
1.  Create a feature branch: `git checkout -b feature/my-feature`
2.  Implement your changes.
3.  Add tests for new functionality.
4.  Verify everything:
    ```bash
    cargo test
    cargo clippy
    cargo fmt --all -- --check
    ```

### 3. Testing with Docker
To ensure your changes work in our CI environment:
```bash
docker build -t labwired-test .
docker run --rm labwired-test
```

### 4. Submitting a Pull Request
- Push your branch to GitHub.
- Open a PR against `main`.
- Ensure all CI checks pass.
- Request review from a maintainer.

## Developer Certificate of Origin (DCO)

All contributions must be signed off under the [Developer Certificate of
Origin 1.1](https://developercertificate.org/). By signing off you certify
that you wrote the contribution (or have the right to submit it) and that it
may be distributed under the project's license.

Add a `Signed-off-by` line to every commit:

```bash
git commit -s -m "feat: my change"
```

This appends:

```
Signed-off-by: Your Name <your.email@example.com>
```

The name and email must match the commit author. CI rejects PRs containing
commits without a sign-off. To fix existing commits:

```bash
git rebase --signoff main && git push --force-with-lease
```

## Coding Standards
- **Style**: We use standard `rustfmt` settings.
- **Linting**: No `clippy` warnings allowed.
- **Documentation**: Public APIs must have doc comments (`///`).
- **Testing**:
    - **Unit Tests**: Add unit tests for all new functions and modules. Run with `cargo test`.
    - **Integration Tests**: Add integration tests in `tests/` or `crates/cli/tests/` for CLI behavior.
    - **SVD Fixtures**: Use `tests/fixtures` for sample SVD files.
    - **Coverage**: We use `tarpaulin` for coverage. Ensuring high coverage is encouraged.


## Adding a New Chip

To add support for a new MCU or board, follow the step-by-step checklist in
[docs/guides/onboarding-a-chip.md](docs/guides/onboarding-a-chip.md).
It covers every touchpoint (sim model, pin-mapping, board catalog, editor
component) and the CI tests that enforce the procedure.

## Reporting Issues
Please open an issue on GitHub describing the bug or feature request clearly.
