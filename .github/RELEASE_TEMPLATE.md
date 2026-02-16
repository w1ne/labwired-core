# Release vX.Y.Z: <One-line summary>

## Scope
- Components: `<core | vscode | ai | docs>`
- Release date: `YYYY-MM-DD`
- Commit: `<sha>`

## Highlights
- <Top change 1>
- <Top change 2>
- <Top change 3>

## Improvements
- <Improvement 1>
- <Improvement 2>

## Fixes
- <Fix 1>
- <Fix 2>

## Documentation Updates
- Core docs: <updated files or "No docs changes (reason)">
- VS Code docs: <updated files or "No docs changes (reason)">
- AI docs: <updated files or "No docs changes (reason)">
- Root/docs: <updated files or "No docs changes (reason)">

## Breaking Changes
- None.

## Validation Evidence
### Local Gates
- `cargo fmt --all -- --check`: PASS
- `cargo clippy --workspace --all-targets -- -D warnings`: PASS
- `cargo test --workspace`: PASS
- `cargo build --workspace`: PASS

### Instruction Coverage Gate
- Unsupported instruction audit command: PASS
- Report: `core/out/unsupported-audit/<target>/report.md`
- Unsupported instruction count: `0`

### Component Gates (as applicable)
- VS Code compile/tests: PASS
- AI demo dry-run: PASS

### CI
- Rust Core CI: `<url>`
- VS Code Extension CI: `<url>`

## Artifacts
- `<artifact name + path/link>`

## Known Issues
- None.

## Upgrade Notes
- <Any required migration steps; otherwise "No special upgrade steps.">
