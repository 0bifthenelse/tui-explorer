#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat <<'EOF'
Usage: install.sh [--prefix DIR] [--help]

Builds tui-explorer in release mode and installs the binary.
Safe to re-run: an existing install at the target path is overwritten
seamlessly, with no prompt and no error.

  --prefix DIR   install under DIR/bin instead of the automatic default
  -h, --help     show this help and exit

Automatic default: /usr/local (system-wide) when run as root,
otherwise $HOME/.local (user-local, XDG convention).
EOF
}

PREFIX=""
while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix)
            [ "$#" -ge 2 ] || { echo "install.sh: --prefix requires an argument" >&2; exit 1; }
            PREFIX="$2"
            shift 2
            ;;
        --prefix=*)
            PREFIX="${1#--prefix=}"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "install.sh: unknown argument: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

if [ -z "$PREFIX" ]; then
    if [ "$(id -u)" -eq 0 ]; then
        PREFIX="/usr/local"
    else
        PREFIX="$HOME/.local"
    fi
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

if ! command -v cargo >/dev/null 2>&1; then
    echo "install.sh: cargo not found. Install a Rust toolchain (see README.md 'Installation')." >&2
    exit 1
fi

echo "Building tui-explorer (release)..."
cargo build --release --locked --bin tui-explorer

BIN_SRC="$SCRIPT_DIR/target/release/tui-explorer"
if [ ! -x "$BIN_SRC" ]; then
    echo "install.sh: build did not produce $BIN_SRC" >&2
    exit 1
fi

BIN_DIR="$PREFIX/bin"
DEST="$BIN_DIR/tui-explorer"

if [ -e "$DEST" ]; then
    echo "Existing install found at $DEST, overwriting."
fi

install -Dm755 "$BIN_SRC" "$DEST"

if ! command -v mpv >/dev/null 2>&1; then
    echo "Note: mpv is not installed. Direct video playback requires mpv with a Kitty graphics terminal."
    echo "Audio playback and all other features work without it."
fi

echo "Installed tui-explorer to $DEST"

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
        echo "Warning: $BIN_DIR is not on your PATH."
        echo "Add it, e.g.: echo 'export PATH=\"$BIN_DIR:\$PATH\"' >> ~/.bashrc"
        ;;
esac

"$DEST" --version
