# Windows Simulator Release Design

## Outcome

LabWired Core releases will publish a native `windows-x86_64` `labwired.exe` archive alongside the existing macOS and Linux CLI archives. The archive is built from the same tagged source and contains only the release CLI. It gives the LabWired Agent and desktop sidecar a real owned simulator input on Windows instead of a fake test adapter or a user-installed fallback.

## Scope and boundary

The first supported Windows architecture is `x86_64-pc-windows-msvc`, built on `windows-latest`. It is the architecture used by the initial desktop installer and the existing agent installer already recognizes `labwired-vX.Y.Z-windows-x86_64.zip`.

Windows ARM64 is deliberately out of scope for this change. It requires a separate native/cross-link validation and must not be advertised by the desktop sidecar manifest before that proof exists. Code signing, installer packaging, desktop resource embedding, and updater delivery are downstream tasks; this change creates the versioned simulator artifact they consume.

## Release shape

Each tag build retains the current archives:

- `labwired-vX.Y.Z-linux-x86_64.tar.gz`
- `labwired-vX.Y.Z-linux-aarch64.tar.gz`
- `labwired-vX.Y.Z-darwin-x86_64.tar.gz`
- `labwired-vX.Y.Z-darwin-aarch64.tar.gz`

and adds:

- `labwired-vX.Y.Z-windows-x86_64.zip`, containing one `labwired.exe`.

After collecting every platform artifact, the release job writes and uploads a `SHA256SUMS` file generated from the exact archive bytes. Future agent and desktop sidecar code pins an asset URL and its digest from this release; it never resolves `latest` at customer runtime.

## Verification model

Pull requests gain a native Windows CLI job. It builds the release CLI, runs `labwired.exe --version`, packages a ZIP in a temporary directory, extracts it, and runs the extracted executable again. That makes the release archive shape testable before a tag.

The tag release workflow independently builds the same target, names the asset deterministically, publishes it with the other archives, and publishes checksums. GitHub Actions is the authoritative Windows execution environment for this task; macOS cannot validate the Windows binary locally.

## Downstream contract

The agent’s real simulator CI gate remains Linux-only until the Windows archive is released. Once a tagged release includes the Windows ZIP and checksum, the desktop sidecar manifest must declare the exact version/digest per platform and enable local Windows virtual-target capability only when its resource integrity check passes. A missing Windows asset is a hard packaging failure, not a reason to expose a nonfunctional local target.
