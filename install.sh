#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: ./install.sh [--prefix PATH] [--bin-name NAME]

Builds Ftop in release mode and installs the binary to PATH/bin.

Options:
  --prefix PATH   Install prefix directory (default: /usr/local)
  --bin-name NAME Installed binary name (default: ftop)
  -h, --help      Show this help message
EOF
}

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$script_dir"
prefix="${PREFIX:-/usr/local}"
bin_name="ftop"

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

echo "Building release binary..."
# Build the specific binary requested by --bin-name to ensure the
# produced artifact matches `$bin_name`.
cargo build --release --locked --manifest-path "$repo_root/Cargo.toml" --bin "$bin_name"

if [[ ! -f "$target_bin" ]]; then
  echo "error: built binary not found at $target_bin" >&2
  exit 1
fi

echo "Installing to $install_path"

# Create destination directory first (portable across platforms)
if mkdir -p "$install_dir" 2>/dev/null; then
  # Prefer 'install' to set mode, but not all platforms support '-D'
  if install -m755 "$target_bin" "$install_path" 2>/dev/null; then
    echo "ftop installed successfully"
    exit 0
  fi

  # Fallback to copy + chmod
  if cp "$target_bin" "$install_path" 2>/dev/null && chmod 755 "$install_path" 2>/dev/null; then
    echo "ftop installed successfully"
    exit 0
  fi
fi

# Try with sudo if we couldn't write the destination
if command -v sudo >/dev/null 2>&1; then
  sudo mkdir -p "$install_dir" 2>/dev/null || true
    if sudo install -m755 "$target_bin" "$install_path" 2>/dev/null; then
    echo "ftop installed successfully"
    exit 0
  fi
    if sudo cp "$target_bin" "$install_path" 2>/dev/null && sudo chmod 755 "$install_path" 2>/dev/null; then
    echo "ftop installed successfully"
    exit 0
  fi
fi

echo "error: failed to install $target_bin to $install_path" >&2
echo "hint: rerun with --prefix <writable-path> or install sudo" >&2
exit 1