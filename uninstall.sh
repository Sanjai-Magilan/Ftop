#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./uninstall.sh [--prefix PATH] [--bin-name NAME]

Removes the installed SysWatcher binary from PATH/bin.

Options:
  --prefix PATH   Install prefix directory (default: /usr/local)
  --bin-name NAME Installed binary name (default: syswatcher)
  -h, --help      Show this help message
EOF
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$script_dir"
prefix="${PREFIX:-/usr/local}"
bin_name="syswatcher"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --prefix)
      [[ $# -ge 2 ]] || { echo "error: --prefix requires a value" >&2; exit 1; }
      prefix="$2"
      shift 2
      ;;
    --bin-name)
      [[ $# -ge 2 ]] || { echo "error: --bin-name requires a value" >&2; exit 1; }
      bin_name="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

install_dir="$prefix/bin"
install_path="$install_dir/$bin_name"

if [[ ! -e "$install_path" ]]; then
  echo "Nothing to do: $install_path does not exist"
  exit 0
fi

echo "Removing $install_path"
if rm -f "$install_path" 2>/dev/null; then
  echo "Removed $install_path"
  exit 0
fi

if command -v sudo >/dev/null 2>&1; then
  if sudo rm -f "$install_path"; then
    echo "Removed $install_path (via sudo)"
    exit 0
  else
    echo "error: failed to remove $install_path even with sudo" >&2
    exit 1
  fi
fi

echo "error: unable to remove $install_path and sudo is not available" >&2
echo "hint: rerun with --prefix <writable-path> or install sudo" >&2
exit 1
