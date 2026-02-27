#!/usr/bin/env sh
# LabWired CLI Installer
# Usage: curl -fsSL https://labwired.com/install.sh | sh
#
# Options (set via env vars):
#   LABWIRED_VERSION=latest          - specific version tag, e.g. "v0.12.0"
#   LABWIRED_INSTALL_DIR=~/.local/bin - install directory
#   LABWIRED_NO_MODIFY_PATH=1        - skip adding to PATH in shell rc
#   LABWIRED_FROM_SOURCE=1           - skip prebuilt, always build from source
#
# MIT License - Copyright (C) 2026 LabWired

set -eu

REPO="w1ne/labwired-core"
BINARY_NAME="labwired"
INSTALL_DIR="${LABWIRED_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${LABWIRED_VERSION:-latest}"
FROM_SOURCE="${LABWIRED_FROM_SOURCE:-0}"

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
die()   { printf '%s  %s%s\n' "${red}✗${rst}" "$*" "${rst}" >&2; exit 1; }
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
}

# ── Helpers ────────────────────────────────────────────────────────────────────
need_cmd() { command -v "$1" >/dev/null 2>&1 || die "Required command not found: $1 — please install it and retry."; }

check_downloader() {
  if command -v curl >/dev/null 2>&1; then
    DOWNLOADER="curl"
  elif command -v wget >/dev/null 2>&1; then
    DOWNLOADER="wget"
  else
    die "Neither curl nor wget found. Please install one and retry."
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

resolve_version() {
  if [ "$VERSION" = "latest" ]; then
    info "Resolving latest version..."
    _api="https://api.github.com/repos/${REPO}/releases/latest"
    _json="$(download_stdout "$_api" 2>/dev/null)" || true
    VERSION="$(printf '%s' "$_json" | grep '"tag_name"' | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')"
    if [ -z "$VERSION" ]; then
      warn "Could not resolve latest version from GitHub API — will build from source."
      VERSION=""
    fi
  fi
}

# ── Prebuilt install ───────────────────────────────────────────────────────────
install_prebuilt() {
  _platform="$1"
  _version="$2"
  _archive="${BINARY_NAME}-${_version}-${_platform}.tar.gz"
  _url="https://github.com/${REPO}/releases/download/${_version}/${_archive}"

  info "Downloading prebuilt binary: ${_archive}"

  _tmpdir="$(mktemp -d)"
  _archive_path="${_tmpdir}/${_archive}"

  if ! download "$_url" "$_archive_path" 2>/dev/null; then
    rm -rf "$_tmpdir"
    return 1
  fi

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

  ensure_rust
  info "Building labwired from source (this takes a few minutes)..."

  # shellcheck disable=SC2086
  cargo install --locked \
    --git "https://github.com/${REPO}" \
    ${_version_arg} \
    labwired-cli \
    --root "$INSTALL_DIR/.."

  # cargo install puts into {root}/bin — adjust INSTALL_DIR for PATH message
  INSTALL_DIR="${INSTALL_DIR%/bin}/.cargo/bin"
  [ -d "$INSTALL_DIR" ] || INSTALL_DIR="${HOME}/.cargo/bin"
  ok "Built and installed ${BINARY_NAME} → ${INSTALL_DIR}/${BINARY_NAME}"
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
    if [ -n "$PLATFORM" ] && [ -n "$VERSION" ]; then
      install_prebuilt "$PLATFORM" "$VERSION" && _prebuilt_ok=1 || true
    fi

    if [ "$_prebuilt_ok" = "0" ]; then
      if [ -z "$PLATFORM" ]; then
        warn "Unsupported platform ($(uname -s)/$(uname -m)) — falling back to source build."
      else
        warn "No prebuilt binary available for ${PLATFORM} ${VERSION} — falling back to source build."
      fi
      install_from_source
    fi
  fi

  add_to_path

  printf '\n'
  printf '%s  Installation complete!%s\n' "${bld}${cyn}" "${rst}"
  printf '\n'
  printf '  Run:  %s%s --version%s\n'      "$bld" "$BINARY_NAME" "$rst"
  printf '  Docs: %shttps://labwired.com/docs/%s\n\n' "$cyn" "$rst"

  if ! command -v "$BINARY_NAME" >/dev/null 2>&1; then
    warn "Restart your shell or run:  source ~/.bashrc  (or ~/.zshrc)"
  fi
}

main "$@"
