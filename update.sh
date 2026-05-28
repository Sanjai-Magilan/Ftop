#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./update.sh [--prefix PATH] [--bin-name NAME]

Rebuilds SysWatcher and updates an existing installation in PATH/bin.

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

target_bin="$repo_root/target/release/$bin_name"
install_dir="$prefix/bin"
install_path="$install_dir/$bin_name"

if [[ ! -e "$install_path" ]]; then
  echo "error: no existing installation found at $install_path" >&2
  echo "hint: run ./install.sh first" >&2
  exit 1
fi

echo "Building release binary..."
cargo build --release --locked --manifest-path "$repo_root/Cargo.toml" --bin "$bin_name"

if [[ ! -f "$target_bin" ]]; then
  echo "error: built binary not found at $target_bin" >&2
  exit 1
fi

echo "Updating $install_path"

if mkdir -p "$install_dir" 2>/dev/null; then
  if install -m755 "$target_bin" "$install_path" 2>/dev/null; then
    echo "system watcher updated successfully"
    exit 0
  fi

  if cp "$target_bin" "$install_path" 2>/dev/null && chmod 755 "$install_path" 2>/dev/null; then
    echo "system watcher updated successfully"
    exit 0
  fi
fi

if command -v sudo >/dev/null 2>&1; then
  sudo mkdir -p "$install_dir" 2>/dev/null || true
  if sudo install -m755 "$target_bin" "$install_path" 2>/dev/null; then
    echo "system watcher updated successfully"
    exit 0
  fi
  if sudo cp "$target_bin" "$install_path" 2>/dev/null && sudo chmod 755 "$install_path" 2>/dev/null; then
    echo "system watcher updated successfully"
    exit 0
  fi
fi

echo "error: failed to update $install_path" >&2
echo "hint: rerun with --prefix <writable-path> or install sudo" >&2
exit 1
