#!/usr/bin/env sh
# LabWired CLI Installer
# Usage: curl -fsSL https://labwired.com/install.sh | sh
#
# Options (set via env vars):
#   LABWIRED_VERSION=latest          - specific version tag, e.g. "v0.14.0"
#   LABWIRED_INSTALL_DIR=~/.local/bin - install directory
#   LABWIRED_NO_MODIFY_PATH=1        - skip adding to PATH in shell rc
#   LABWIRED_FROM_SOURCE=1           - skip prebuilt, always build from source
#   LABWIRED_TELEMETRY=0             - do not report install failures (DO_NOT_TRACK also honoured)
#
# MIT License - Copyright (C) 2026 LabWired

set -eu

REPO="w1ne/labwired-core"
BINARY_NAME="labwired"
DAP_BINARY_NAME="labwired-dap"
INSTALL_DIR="${LABWIRED_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${LABWIRED_VERSION:-latest}"
FROM_SOURCE="${LABWIRED_FROM_SOURCE:-0}"
TELEMETRY_URL="${LABWIRED_TELEMETRY_URL:-https://api.labwired.com/v1/telemetry/failure}"
PLATFORM=""
HOST_CLASS=""
STAGE="start"

# ── Colours ────────────────────────────────────────────────────────────────────
red=""
grn=""
ylw=""
cyn=""
bld=""
rst=""
if [ -t 1 ] && command -v tput >/dev/null 2>&1; then
  red="$(tput setaf 1)"
  grn="$(tput setaf 2)"
  ylw="$(tput setaf 3)"
  cyn="$(tput setaf 6)"
  bld="$(tput bold)"
  rst="$(tput sgr0)"
fi

info()  { printf '%s  %s%s\n' "${cyn}→${rst}" "$*" "${rst}"; }
ok()    { printf '%s  %s%s\n' "${grn}✓${rst}" "$*" "${rst}"; }
warn()  { printf '%s  %s%s\n' "${ylw}!${rst}" "$*" "${rst}"; }

# ── Failure reporting ──────────────────────────────────────────────────────────
# Every exit path reports one enumerated field set — no paths, no user data, no
# free text. This is the only signal we have that an install broke on a machine
# we do not own: the 2026-08 outage (a `latest` tag with no CLI archive, and a
# Linux binary that needed a newer glibc than the distro shipped) was invisible
# for two weeks because a failing install told nobody. `$1` is the event, `$2`
# the enumerated class. Never blocks the install: 2 s budget, failures ignored.
beacon() {
  if [ "${LABWIRED_TELEMETRY:-1}" = "0" ] || [ -n "${DO_NOT_TRACK:-}" ]; then
    return 0
  fi
  if ! command -v curl >/dev/null 2>&1; then
    return 0
  fi
  curl -fsS -m 2 -X POST "$TELEMETRY_URL" \
    -H 'content-type: application/json' \
    -d "{\"surface\":\"installer\",\"event\":\"$1\",\"stage\":\"${STAGE}\",\
\"error_class\":\"$2\",\"platform\":\"${PLATFORM:-unknown}\",\
\"release\":\"${VERSION:-unknown}\",\"host_class\":\"${HOST_CLASS:-unknown}\",\
\"channel\":\"install.sh\"}" >/dev/null 2>&1 || true
  return 0
}

# `die <error_class> <message>` — the class is enumerated and reported, the
# message is for the human in front of the terminal only.
die() {
  _class="$1"
  shift
  beacon install_fail "$_class"
  printf '%s  %s%s\n' "${red}✗${rst}" "$*" "${rst}" >&2
  exit 1
}
# ── Banner ─────────────────────────────────────────────────────────────────────
print_banner() {
  _c="${cyn}"
  _r="${rst}"
  _b="${bld}"
  _y="${ylw}"

  printf '\n'
  sleep 0.1
  printf '%s ██╗      █████╗ ██████╗ ██╗    ██╗██╗██████╗ ███████╗██████╗%s\n'          "$_c" "$_r"; sleep 0.1
  printf '%s ██║     ██╔══██╗██╔══██╗██║    ██║██║██╔══██╗██╔════╝██╔══██╗%s\n'         "$_c" "$_r"; sleep 0.1
  printf '%s ██║     ███████║██████╔╝██║ █╗ ██║██║██████╔╝█████╗  ██║  ██║%s\n'         "$_c" "$_r"; sleep 0.1
  printf '%s ██║     ██╔══██║██╔══██╗██║███╗██║██║██╔══██╗██╔══╝  ██║  ██║%s\n'         "$_c" "$_r"; sleep 0.1
  printf '%s ███████╗██║  ██║██████╔╝╚███╔███╔╝██║██║  ██║███████╗██████╔╝%s\n'         "$_c" "$_r"; sleep 0.1
  printf '%s ╚══════╝╚═╝  ╚═╝╚═════╝  ╚══╝╚══╝ ╚═╝╚═╝  ╚═╝╚══════╝╚═════╝%s\n'        "$_c" "$_r"; sleep 0.15
  printf '\n'
  printf '  %sfirmware simulation engine%s\n'                  "$_b" "$_r"
  printf '  %sinspect · test · debug — first in simulation%s\n\n' "$_y" "$_r"
}

print_banner

# ── OS / Arch detection ────────────────────────────────────────────────────────
detect_platform() {
  _os="$(uname -s)"
  _arch="$(uname -m)"

  case "$_os" in
    Linux)  _os_tag="linux"  ;;
    Darwin) _os_tag="darwin" ;;
    *)      _os_tag="" ;;
  esac

  case "$_arch" in
    x86_64|amd64)         _arch_tag="x86_64"   ;;
    aarch64|arm64)        _arch_tag="aarch64"  ;;
    *)                    _arch_tag="" ;;
  esac

  if [ -n "$_os_tag" ] && [ -n "$_arch_tag" ]; then
    PLATFORM="${_os_tag}-${_arch_tag}"
  else
    PLATFORM=""
  fi

  # The host's C library / OS version. This is the field that names a
  # "binary installed but will not start" failure without a bug report.
  if [ "$_os_tag" = "linux" ]; then
    HOST_CLASS="glibc-$(ldd --version 2>/dev/null | head -1 | grep -oE '[0-9]+\.[0-9]+$' || echo unknown)"
  elif [ "$_os_tag" = "darwin" ]; then
    HOST_CLASS="macos-$(sw_vers -productVersion 2>/dev/null | cut -d. -f1 || echo unknown)"
  else
    HOST_CLASS="unknown"
  fi
}

# ── Helpers ────────────────────────────────────────────────────────────────────
need_cmd() { command -v "$1" >/dev/null 2>&1 || die missing_command "Required command not found: $1 — please install it and retry."; }

check_downloader() {
  if command -v curl >/dev/null 2>&1; then
    DOWNLOADER="curl"
  elif command -v wget >/dev/null 2>&1; then
    DOWNLOADER="wget"
  else
    die no_downloader "Neither curl nor wget found. Please install one and retry."
  fi
}

download() {
  _url="$1"
  _dest="$2"
  if [ "$DOWNLOADER" = "curl" ]; then
    curl -fsSL --retry 3 -o "$_dest" "$_url"
  else
    wget -qO "$_dest" "$_url"
  fi
}

download_stdout() {
  _url="$1"
  if [ "$DOWNLOADER" = "curl" ]; then
    curl -fsSL --retry 3 "$_url"
  else
    wget -qO- "$_url"
  fi
}

# Resolve "latest" to the newest CLI release — and ONLY to a CLI release.
#
# `/releases/latest` returns the newest non-prerelease of ANY kind. This repo
# also publishes asset-only releases (`firmware-demos-vN`, playground demo
# ELFs), and on 2026-08-01 one of those became "latest": every unpinned
# `curl … | sh` then asked for labwired-firmware-demos-v3-<platform>.tar.gz,
# got a 404, and silently escalated to a from-source build. So filter the
# release list to vMAJOR.MINOR.PATCH here rather than trusting the endpoint —
# the same guard `.github/actions/labwired-test/action.yml` already applies to
# its `version` input.
resolve_version() {
  if [ "$VERSION" = "latest" ]; then
    STAGE="resolve"
    info "Resolving latest version..."

    # First choice is the web redirect, NOT the API. `github.com/<repo>/releases/latest`
    # redirects to `/releases/tag/<tag>`, needs no token, and — unlike a bare
    # API call — is not rate limited per IP. That matters more than it sounds:
    # the API allows 60 unauthenticated requests an hour per address, so on a
    # shared address (CI, an office, a container host) an install can fail for
    # reasons that have nothing to do with this machine. The redirect also
    # excludes prereleases, which is how demo-firmware tags stay out of it.
    _tag=""
    if [ "$DOWNLOADER" = "curl" ]; then
      _tag="$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
        "https://github.com/${REPO}/releases/latest" 2>/dev/null | sed 's#.*/tag/##')"
    fi
    case "$_tag" in
      v[0-9]*.[0-9]*.[0-9]*) VERSION="$_tag" ;;
      *) _tag="" ;;
    esac

    # Fall back to the release list, filtered to CLI releases. Reached when the
    # redirect is unavailable (wget, a proxy that eats redirects) or points at
    # something that is not a CLI release.
    if [ -z "$_tag" ]; then
      _api="https://api.github.com/repos/${REPO}/releases?per_page=50"
      _json="$(download_stdout "$_api" 2>/dev/null)" || true
      VERSION="$(printf '%s' "$_json" \
        | sed -n 's/.*"tag_name": *"\(v[0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\)".*/\1/p' \
        | head -1)"
    fi

    if [ -z "$VERSION" ] || [ "$VERSION" = "latest" ]; then
      die no_cli_release "Could not resolve a CLI release (vMAJOR.MINOR.PATCH).
     GitHub may be rate limiting this address. Pin a version instead:
       curl -fsSL https://labwired.com/install.sh | LABWIRED_VERSION=v0.21.0 sh"
    fi
  else
    case "$VERSION" in
      v[0-9]*.[0-9]*.[0-9]*) : ;;
      *) die bad_version_pin "LABWIRED_VERSION must be a release tag like v0.21.0, got: ${VERSION}" ;;
    esac
  fi
}

# ── Prebuilt install ───────────────────────────────────────────────────────────
install_prebuilt() {
  _platform="$1"
  _version="$2"
  _archive="${BINARY_NAME}-${_version}-${_platform}.tar.gz"
  _url="https://github.com/${REPO}/releases/download/${_version}/${_archive}"

  STAGE="download"
  info "Downloading prebuilt binary: ${_archive}"

  _tmpdir="$(mktemp -d)"
  _archive_path="${_tmpdir}/${_archive}"

  if ! download "$_url" "$_archive_path" 2>/dev/null; then
    rm -rf "$_tmpdir"
    return 1
  fi

  STAGE="extract"
  tar -xzf "$_archive_path" -C "$_tmpdir"
  rm "$_archive_path"

  _extracted="${_tmpdir}/${BINARY_NAME}"
  if [ ! -f "$_extracted" ]; then
    # Some archives nest in a subdirectory
    _extracted="$(find "$_tmpdir" -name "$BINARY_NAME" -type f | head -1)"
  fi

  if [ -z "$_extracted" ] || [ ! -f "$_extracted" ]; then
    rm -rf "$_tmpdir"
    return 1
  fi

  mkdir -p "$INSTALL_DIR"
  cp "$_extracted" "${INSTALL_DIR}/${BINARY_NAME}"
  chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

  # labwired-dap is the Debug Adapter the VS Code extension spawns for F5.
  # Optional: tarballs from before v0.19.3 only carry the CLI, so a missing
  # binary here means "older release", not a failed install.
  _dap="${_tmpdir}/${DAP_BINARY_NAME}"
  if [ ! -f "$_dap" ]; then
    _dap="$(find "$_tmpdir" -name "$DAP_BINARY_NAME" -type f | head -1)"
  fi
  if [ -n "$_dap" ] && [ -f "$_dap" ]; then
    cp "$_dap" "${INSTALL_DIR}/${DAP_BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${DAP_BINARY_NAME}"
    ok "Installed prebuilt ${DAP_BINARY_NAME} ${_version} → ${INSTALL_DIR}/${DAP_BINARY_NAME}"
  fi

  rm -rf "$_tmpdir"
  ok "Installed prebuilt ${BINARY_NAME} ${_version} → ${INSTALL_DIR}/${BINARY_NAME}"
  return 0
}

# ── Source install via cargo ───────────────────────────────────────────────────
ensure_rust() {
  if command -v cargo >/dev/null 2>&1; then
    ok "Rust toolchain found: $(rustc --version 2>/dev/null || echo 'unknown')"
    return
  fi

  warn "Rust toolchain not found — installing via rustup..."
  check_downloader
  _rustup_sh="$(mktemp)"
  download "https://sh.rustup.rs" "$_rustup_sh"
  chmod +x "$_rustup_sh"
  sh "$_rustup_sh" -y --no-modify-path
  rm "$_rustup_sh"

  # Source cargo env for the rest of this script
  # shellcheck disable=SC1090
  . "${HOME}/.cargo/env" 2>/dev/null || export PATH="${HOME}/.cargo/bin:${PATH}"
  ok "Rust installed: $(rustc --version 2>/dev/null)"
}

install_from_source() {
  _version_arg=""
  if [ -n "$VERSION" ] && [ "$VERSION" != "latest" ]; then
    _version_arg="--tag ${VERSION}"
  fi

  STAGE="build"
  ensure_rust
  info "Building labwired from source (this takes a few minutes)..."

  # shellcheck disable=SC2086
  cargo install --locked \
    --git "https://github.com/${REPO}" \
    ${_version_arg} \
    labwired-cli labwired-dap \
    --root "$INSTALL_DIR/.."

  # cargo install puts into {root}/bin — adjust INSTALL_DIR for PATH message
  INSTALL_DIR="${INSTALL_DIR%/bin}/.cargo/bin"
  [ -d "$INSTALL_DIR" ] || INSTALL_DIR="${HOME}/.cargo/bin"
  ok "Built and installed ${BINARY_NAME} → ${INSTALL_DIR}/${BINARY_NAME}"
}

# ── Post-install verification ──────────────────────────────────────────────────
# An installed file is not an installed program. The v0.21.0 Linux archives were
# built on a runner whose glibc was newer than the distros we tell people to use,
# so the copy succeeded, the installer printed "Installation complete!", exited
# 0, and the binary then died with `GLIBC_2.39 not found` the first time anyone
# ran it. Run it here, while we still hold the context to explain what happened.
verify_install() {
  STAGE="verify"
  _bin="${INSTALL_DIR}/${BINARY_NAME}"
  [ -x "$_bin" ] || die missing_binary "Install finished but ${_bin} is not there."

  if _out="$("$_bin" --version 2>&1)"; then
    ok "Verified: ${_out}"
    return 0
  fi

  case "$_out" in
    *GLIBC*|*"libc.so"*|*"not found"*)
      die glibc_missing "The installed binary cannot start on this system:
       ${_out}
     This build needs a newer C library than this distribution provides.
     Please report it with your distro version: https://github.com/${REPO}/issues" ;;
    *)
      die binary_wont_run "The installed binary cannot start on this system:
       ${_out}" ;;
  esac
}

# ── PATH setup ─────────────────────────────────────────────────────────────────
add_to_path() {
  if [ "${LABWIRED_NO_MODIFY_PATH:-0}" = "1" ]; then
    return
  fi

  case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) return ;;
  esac

  _export_line="export PATH=\"${INSTALL_DIR}:\$PATH\""

  for _rc in "${HOME}/.bashrc" "${HOME}/.zshrc" "${HOME}/.profile"; do
    if [ -f "$_rc" ]; then
      if ! grep -qF "$INSTALL_DIR" "$_rc" 2>/dev/null; then
        printf '\n# Added by LabWired installer\n%s\n' "$_export_line" >> "$_rc"
        ok "Added ${INSTALL_DIR} to PATH in ${_rc}"
      fi
    fi
  done
}

# ── Main ───────────────────────────────────────────────────────────────────────
main() {
  check_downloader
  detect_platform

  # Decide install strategy
  if [ "$FROM_SOURCE" = "1" ] || [ "$FROM_SOURCE" = "true" ]; then
    info "Source install requested (LABWIRED_FROM_SOURCE=1)"
    resolve_version
    install_from_source
  else
    resolve_version

    # Try prebuilt first
    _prebuilt_ok=0
    if [ -n "$PLATFORM" ]; then
      install_prebuilt "$PLATFORM" "$VERSION" && _prebuilt_ok=1 || true
    fi

    # A missing archive used to fall through to `install_from_source`, which
    # downloads a whole Rust toolchain and compiles the engine — minutes of
    # work, gigabytes of disk, and none of it asked for. Building from source
    # is a fine answer, but it is the user's answer to give.
    if [ "$_prebuilt_ok" = "0" ]; then
      if [ -z "$PLATFORM" ]; then
        die unsupported_platform "No prebuilt binary for $(uname -s)/$(uname -m).
     Build it yourself (installs Rust, takes a few minutes):
       curl -fsSL https://labwired.com/install.sh | LABWIRED_FROM_SOURCE=1 sh"
      fi
      die asset_404 "No prebuilt binary for ${PLATFORM} at ${VERSION}.
     Pick another release:  https://github.com/${REPO}/releases
     Or build from source:  curl -fsSL https://labwired.com/install.sh | LABWIRED_FROM_SOURCE=1 sh"
    fi
  fi

  verify_install
  add_to_path

  printf '\n'
  printf '%s  Installation complete!%s\n' "${bld}${cyn}" "${rst}"
  printf '\n'
  printf '  Run:  %s%s --version%s\n'      "$bld" "$BINARY_NAME" "$rst"
  printf '  Docs: %shttps://docs.labwired.com/%s\n\n' "$cyn" "$rst"

  if ! command -v "$BINARY_NAME" >/dev/null 2>&1; then
    warn "Restart your shell or run:  source ~/.bashrc  (or ~/.zshrc)"
  fi
}

main "$@"
