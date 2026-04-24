# Contributing to LabWired Core

First off, thank you for considering contributing to LabWired! It's people like you that make LabWired such a great tool.

## How Can I Contribute?

### Reporting Bugs
- Use the **Bug Report** template.
- Describe the exact steps to reproduce the issue.
- Include your `system.yaml` and the firmware version if applicable.

### Suggesting Enhancements
- Use the **Feature Request** template.
- Explain why this enhancement would be useful to most LabWired users.

### Adding New Architectures or Peripherals
- We love new hardware support!
- For zero-code peripherals, see [Declarative Peripherals](docs/declarative_peripherals.md).
- For custom Rust peripherals, read `crates/core/src/peripherals/*.rs` — every
  peripheral in-tree is a worked example of the `Peripheral` trait.
- Ensure you include unit tests for new peripherals.

## Style Guide
- Run `cargo fmt` before committing.
- Ensure `cargo clippy` passes without warnings.
- Write clear, concise commit messages.

## Pull Request Process
1. Create a branch from `develop`.
2. Ensure the CI suite passes.
3. Update relevant documentation.
4. Your PR will be reviewed by the maintainers.

## License
By contributing, you agree that your contributions will be licensed under the MIT License.
