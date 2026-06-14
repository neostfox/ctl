#!/usr/bin/env sh
# ctl installer (Linux / macOS)
#
#   curl -fsSL https://raw.githubusercontent.com/neostfox/ctl/master/scripts/install.sh | sh
#
# Options (flags or environment variables):
#   --version <vX.Y.Z>   CTL_VERSION       release tag to install (default: latest)
#   --dir <path>         CTL_INSTALL_DIR   install directory (default: /usr/local/bin or ~/.local/bin)
set -eu

REPO="neostfox/ctl"
BIN="ctl"
VERSION="${CTL_VERSION:-latest}"
INSTALL_DIR="${CTL_INSTALL_DIR:-}"

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    --dir)     INSTALL_DIR="$2"; shift 2 ;;
    -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
    *) echo "ctl-install: unknown argument: $1" >&2; exit 2 ;;
  esac
done

err() { echo "ctl-install: $*" >&2; exit 1; }

os="$(uname -s)"
arch="$(uname -m)"
case "$os" in
  Linux)  os_t="unknown-linux-gnu" ;;
  Darwin) os_t="apple-darwin" ;;
  *) err "unsupported OS: $os" ;;
esac
case "$arch" in
  x86_64|amd64)   arch_t="x86_64" ;;
  aarch64|arm64)  arch_t="aarch64" ;;
  *) err "unsupported architecture: $arch" ;;
esac

target="${arch_t}-${os_t}"
asset="ctl-${target}.tar.gz"

if [ "$VERSION" = "latest" ]; then
  base="https://github.com/${REPO}/releases/latest/download"
else
  base="https://github.com/${REPO}/releases/download/${VERSION}"
fi
url="${base}/${asset}"

if [ -z "$INSTALL_DIR" ]; then
  if [ -w "/usr/local/bin" ] 2>/dev/null; then
    INSTALL_DIR="/usr/local/bin"
  else
    INSTALL_DIR="${HOME}/.local/bin"
  fi
fi
mkdir -p "$INSTALL_DIR"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fetch() { # url out
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO "$2" "$1"
  else
    err "need curl or wget to download"
  fi
}

echo "ctl-install: downloading ${url}"
fetch "$url" "${tmp}/${asset}" || err "download failed: ${url}"
fetch "${url}.sha256" "${tmp}/${asset}.sha256" 2>/dev/null || true

if [ -s "${tmp}/${asset}.sha256" ]; then
  echo "ctl-install: verifying checksum"
  ( cd "$tmp" && {
      if command -v sha256sum >/dev/null 2>&1; then sha256sum -c "${asset}.sha256"
      elif command -v shasum   >/dev/null 2>&1; then shasum -a 256 -c "${asset}.sha256"
      else echo "ctl-install: no sha256 tool, skipping verify" >&2; fi
    } ) || err "checksum verification failed"
fi

tar -xzf "${tmp}/${asset}" -C "$tmp" || err "extract failed"
[ -f "${tmp}/${BIN}" ] || err "binary '${BIN}' not found in archive"

if install -m 0755 "${tmp}/${BIN}" "${INSTALL_DIR}/${BIN}" 2>/dev/null; then :; else
  cp "${tmp}/${BIN}" "${INSTALL_DIR}/${BIN}" && chmod 0755 "${INSTALL_DIR}/${BIN}"
fi

echo "ctl-install: installed to ${INSTALL_DIR}/${BIN}"
case ":${PATH}:" in
  *":${INSTALL_DIR}:"*) ;;
  *) echo "ctl-install: add it to your PATH:"; echo "  export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
esac
"${INSTALL_DIR}/${BIN}" --version 2>/dev/null || true
