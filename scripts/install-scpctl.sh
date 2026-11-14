#!/usr/bin/env bash
set -euo pipefail

ARCHIVE=""
CHECKSUMS=""
PREFIX="${HOME}/.local"

usage() {
  cat <<'EOF'
Usage: install-scpctl.sh --archive <PATH> [--checksums <PATH>] [--prefix <DIR>]
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --archive)
      ARCHIVE="$2"
      shift 2
      ;;
    --checksums)
      CHECKSUMS="$2"
      shift 2
      ;;
    --prefix)
      PREFIX="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [ -z "$ARCHIVE" ]; then
  echo "--archive is required" >&2
  usage >&2
  exit 2
fi

if [ ! -f "$ARCHIVE" ]; then
  echo "Archive not found: $ARCHIVE" >&2
  exit 1
fi

ARCHIVE_DIR="$(cd "$(dirname "$ARCHIVE")" && pwd)"
ARCHIVE_BASE="$(basename "$ARCHIVE")"
ARCHIVE_ABS="${ARCHIVE_DIR}/${ARCHIVE_BASE}"

if [ -z "$CHECKSUMS" ]; then
  CHECKSUMS="${ARCHIVE_DIR}/SHA256SUMS"
fi

if [ ! -f "$CHECKSUMS" ]; then
  echo "Checksums file not found: $CHECKSUMS" >&2
  exit 1
fi

expected_hash="$(awk -v name="$ARCHIVE_BASE" '$2 == name { print $1; exit }' "$CHECKSUMS")"
if [ -z "$expected_hash" ]; then
  echo "No SHA256SUMS entry found for $ARCHIVE_BASE" >&2
  exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
  actual_hash="$(sha256sum "$ARCHIVE_ABS" | awk '{ print $1 }')"
elif command -v shasum >/dev/null 2>&1; then
  actual_hash="$(shasum -a 256 "$ARCHIVE_ABS" | awk '{ print $1 }')"
else
  echo "Neither sha256sum nor shasum is available for checksum verification" >&2
  exit 1
fi

if [ "$expected_hash" != "$actual_hash" ]; then
  echo "Checksum mismatch for $ARCHIVE_BASE (expected $expected_hash, got $actual_hash)" >&2
  exit 1
fi

WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT

tar -xzf "$ARCHIVE_ABS" -C "$WORK_DIR"
ROOT_DIR="$(find "$WORK_DIR" -mindepth 1 -maxdepth 1 -type d | head -n 1)"
if [ -z "$ROOT_DIR" ] || [ ! -f "${ROOT_DIR}/bin/scpctl" ]; then
  echo "Invalid scpctl archive layout" >&2
  exit 1
fi

mkdir -p \
  "${PREFIX}/bin" \
  "${PREFIX}/share/bash-completion/completions" \
  "${PREFIX}/share/zsh/site-functions" \
  "${PREFIX}/share/fish/vendor_completions.d" \
  "${PREFIX}/share/scpctl/completions"

install -m 0755 "${ROOT_DIR}/bin/scpctl" "${PREFIX}/bin/scpctl"
install -m 0644 "${ROOT_DIR}/completions/scpctl.bash" "${PREFIX}/share/bash-completion/completions/scpctl"
install -m 0644 "${ROOT_DIR}/completions/_scpctl" "${PREFIX}/share/zsh/site-functions/_scpctl"
install -m 0644 "${ROOT_DIR}/completions/scpctl.fish" "${PREFIX}/share/fish/vendor_completions.d/scpctl.fish"
install -m 0644 "${ROOT_DIR}/completions/scpctl.ps1" "${PREFIX}/share/scpctl/completions/scpctl.ps1"

SCPCTL_CONFIG_DIR=/nonexistent/scpctl-install-check "${PREFIX}/bin/scpctl" --version >/dev/null
echo "Installed scpctl to ${PREFIX}/bin/scpctl"
