# Windows Simulator Release Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish a reproducible native Windows x64 LabWired simulator archive and prove its build/package contract in pull-request CI.

**Architecture:** Extend the existing Core tag-release matrix with a `windows-latest` native `x86_64-pc-windows-msvc` build. Keep Unix CLI/debug-adapter tarball packaging unchanged; use a PowerShell ZIP path for `labwired.exe` and `labwired-dap.exe`. Add a Windows pull-request build/package/extract/smoke job and generate `SHA256SUMS` only after all release archives are collected.

**Tech Stack:** Existing Rust `labwired-cli`, GitHub Actions, `windows-latest`, PowerShell `Compress-Archive`/`Expand-Archive`, `sha256sum` on the release job.

---

### Task 1: Prove the native Windows CLI and ZIP shape in PR CI

**Files:**
- Modify: `.github/workflows/core-ci.yml`

- [ ] **Step 1: Add the Windows job before changing tag release packaging**

Add this sibling job after `integrity` in `core-ci.yml`:

```yaml
  windows-cli:
    name: core-windows-cli
    runs-on: windows-latest
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@v4
      - name: Install Rust
        uses: dtolnay/rust-toolchain@1.95.0
      - name: Cache dependencies
        uses: Swatinem/rust-cache@v2
        with:
          shared-key: core-windows-cli
          workspaces: . -> target
      - name: Build release CLI and debug adapter
        run: cargo build -p labwired-cli -p labwired-dap --release --target x86_64-pc-windows-msvc
      - name: Smoke the native executable
        shell: pwsh
        run: |
          $cli = 'target/x86_64-pc-windows-msvc/release/labwired.exe'
          $dap = 'target/x86_64-pc-windows-msvc/release/labwired-dap.exe'
          & $cli --version
      - name: Package and verify the Windows release ZIP
        shell: pwsh
        run: |
          $cli = 'target/x86_64-pc-windows-msvc/release/labwired.exe'
          $dap = 'target/x86_64-pc-windows-msvc/release/labwired-dap.exe'
          $stage = Join-Path $env:RUNNER_TEMP 'labwired-windows-stage'
          $verify = Join-Path $env:RUNNER_TEMP 'labwired-windows-verify'
          $archive = Join-Path $env:RUNNER_TEMP 'labwired-v0.0.0-windows-x86_64.zip'
          New-Item -ItemType Directory -Force -Path $stage, $verify | Out-Null
          Copy-Item $cli (Join-Path $stage 'labwired.exe')
          Copy-Item $dap (Join-Path $stage 'labwired-dap.exe')
          Compress-Archive -Path (Join-Path $stage 'labwired.exe'), (Join-Path $stage 'labwired-dap.exe') -DestinationPath $archive -Force
          Expand-Archive -Path $archive -DestinationPath $verify -Force
          if (!(Test-Path (Join-Path $verify 'labwired.exe')) -or !(Test-Path (Join-Path $verify 'labwired-dap.exe'))) { throw 'missing root-level release binary' }
          & (Join-Path $verify 'labwired.exe') --version
```

- [ ] **Step 2: Verify the new job is active on a pull request**

Push the branch and inspect the `core-windows-cli` Actions job. Expected: native build succeeds, both `--version` calls return zero, and the ZIP contains the executable at its root.

- [ ] **Step 3: Commit the PR gate**

```bash
git add .github/workflows/core-ci.yml
git commit -m "ci: verify Windows simulator archive"
```

### Task 2: Add a deterministic Windows archive to the tag release matrix

**Files:**
- Modify: `.github/workflows/core-release.yml`

- [ ] **Step 1: Add a Windows x64 matrix entry**

Add this entry to `jobs.build.strategy.matrix.include`:

```yaml
          - target: x86_64-pc-windows-msvc
            runner: windows-latest
            platform: windows-x86_64
            binary: labwired.exe
            archive_format: zip
            shell: pwsh
```

Keep the existing Unix CLI/debug-adapter package contents and add
`binary: labwired` and `archive_format: tar.gz`/`shell: bash` to each Unix
entry so platform packaging is explicit rather than inferred.

- [ ] **Step 2: Split package commands by archive format**

Keep the existing Bash CLI/debug-adapter tarball command for Unix builds. Add a PowerShell packaging step for Windows:

```yaml
      - name: Package Windows binary
        if: matrix.archive_format == 'zip'
        shell: pwsh
        run: |
          $version = '${{ github.ref_name }}'
          $archive = "labwired-$version-${{ matrix.platform }}.zip"
          New-Item -ItemType Directory -Force -Path dist | Out-Null
          Copy-Item "target/${{ matrix.target }}/release/${{ matrix.binary }}" 'dist/labwired.exe'
          Copy-Item "target/${{ matrix.target }}/release/labwired-dap.exe" 'dist/labwired-dap.exe'
          Compress-Archive -Path 'dist/labwired.exe', 'dist/labwired-dap.exe' -DestinationPath $archive -Force
          "ARCHIVE=$archive" >> $env:GITHUB_ENV
```

Upload the archive using the existing artifact step. The artifact name must remain `labwired-${{ matrix.platform }}` so the release job can merge it with existing platform artifacts.

- [ ] **Step 3: Generate release checksums after artifact collection**

In the `release` job, after `actions/download-artifact@v4`, add:

```yaml
      - name: Generate archive checksums
        shell: bash
        run: |
          set -euo pipefail
          (
            cd dist
            find . -maxdepth 1 -type f \( -name '*.tar.gz' -o -name '*.zip' \) -printf '%f\0' \
              | sort -z \
              | xargs -0 sha256sum > SHA256SUMS
          )
          test -s dist/SHA256SUMS
```

Change `softprops/action-gh-release` `files:` from `dist/*.tar.gz` to `dist/*` so the Windows ZIP and checksum manifest are both published.

- [ ] **Step 4: Verify release workflow syntax and artifact names**

Run:

```bash
git diff --check
rg -n "windows-x86_64|labwired\.exe|SHA256SUMS|dist/\*" .github/workflows/core-release.yml
```

Expected: the only Windows release target is `x86_64-pc-windows-msvc`; no ARM64 asset is declared; all five platform archive names are deterministic.

- [ ] **Step 5: Commit tag release support**

```bash
git add .github/workflows/core-release.yml
git commit -m "release: publish Windows simulator archive"
```

### Task 3: Align release documentation and verify the host baseline

**Files:**
- Modify: `RELEASE_PROCESS.md`
- Create: `docs/superpowers/specs/2026-08-02-windows-simulator-release-design.md`
- Create: `docs/superpowers/plans/2026-08-02-windows-simulator-release.md`

- [ ] **Step 1: Update release-process artifact expectations**

State that tag releases publish Linux/macOS CLI/debug-adapter tarballs, a Windows x64 ZIP containing `labwired.exe` and `labwired-dap.exe`, and `SHA256SUMS`. State that Windows ARM64 is not a released target until separately validated.

- [ ] **Step 2: Verify the current host baseline remains clean**

Run:

```bash
cargo test -p labwired-cli --no-run
git diff --check
git status --short
```

Expected: the existing host CLI compiles; only planned workflow/document changes are present. Windows native execution is verified by `core-windows-cli` in GitHub Actions, not emulation on macOS.

- [ ] **Step 3: Commit documentation**

```bash
git add RELEASE_PROCESS.md docs/superpowers/specs/2026-08-02-windows-simulator-release-design.md docs/superpowers/plans/2026-08-02-windows-simulator-release.md
git commit -m "docs: define Windows simulator release contract"
```
