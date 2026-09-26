#!/usr/bin/env bash
set -euo pipefail

SCRIPT_NAME="${0##*/}"

usage() {
    cat <<EOF
Usage: $SCRIPT_NAME [--prefix DIR] [--help]

Builds tui-explorer in release mode and installs the binary as
DIR/bin/tui-explorer. Safe to re-run: an existing install at the target
path is overwritten, with no prompt and no error.

  --prefix DIR     install under DIR/bin (may also be given as --prefix=DIR)
  -h, --help       show this help and exit

Default prefix is always \$HOME/.local, for every user including root, so a
normal invocation never touches a system-wide directory. This script never
selects /usr/local on its own; pass --prefix /usr/local for a system-wide
install. Run under sudo (SUDO_USER set) without --prefix, the script refuses
to install into root's home: re-run it as your own user, or pass the
--prefix you actually want.

Examples:
  ./$SCRIPT_NAME
  ./$SCRIPT_NAME --prefix ~/.local
  sudo ./$SCRIPT_NAME --prefix /usr/local
EOF
}

die() {
    echo "$SCRIPT_NAME: $*" >&2
    exit 1
}

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

# Absolute HOME: sudo does not always export a readable HOME.
resolve_home() {
    if [ -n "${HOME:-}" ] && [ -d "$HOME" ]; then
        printf '%s\n' "$HOME"
        return 0
    fi
    if [ "$(id -u)" -eq 0 ] && [ -n "${SUDO_USER:-}" ]; then
        getent passwd "$SUDO_USER" 2>/dev/null | cut -d: -f6 && return 0
    fi
    return 1
}

# Last PATH element wins, so a later .local/bin entry counts as present.
path_has_dir() {
    local needle="$1" entry
    local IFS=:
    for entry in $PATH; do
        if [ "$entry" = "$needle" ]; then
            return 0
        fi
    done
    return 1
}

# Default prefix: $HOME/.local for every user, never /usr/local.
default_prefix() {
    if [ -n "${SUDO_USER:-}" ] && [ "$(id -u)" -eq 0 ]; then
        return 1
    fi
    local home
    home="$(resolve_home)" || return 1
    printf '%s\n' "$home/.local"
}


PREFIX=""
PREFIX_SET=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix)
            [ "$#" -ge 2 ] || die "--prefix requires an argument"
            PREFIX="$2"
            PREFIX_SET=1
            shift 2
            ;;
        --prefix=*)
            PREFIX="${1#--prefix=}"
            [ -n "$PREFIX" ] || die "--prefix requires a non-empty argument"
            PREFIX_SET=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --)
            shift
            [ "$#" -eq 0 ] || die "unexpected argument: $1"
            ;;
        *)
            echo "$SCRIPT_NAME: unknown argument: $1" >&2
            usage >&2
            exit 1
            ;;
    esac
done

# Argument parsing first, so --help wins over any environment complaint.
if [ "$PREFIX_SET" -eq 0 ]; then
    if [ -n "${SUDO_USER:-}" ] && [ "$(id -u)" -eq 0 ]; then
        die "refusing to install under sudo into root's home.
  Re-run this script as $SUDO_USER (without sudo) to get the
  per-user default \$HOME/.local install, or pass an explicit prefix, e.g.:
      ./$SCRIPT_NAME --prefix /usr/local
      ./$SCRIPT_NAME --prefix \$HOME/.local"
    fi
    if ! PREFIX="$(default_prefix)"; then
        die "cannot determine your home directory; pass an explicit --prefix DIR"
    fi
elif [ "${PREFIX#/}" = "$PREFIX" ]; then
    # Relative prefixes resolve against the caller's cwd, not the script dir.
    PREFIX="$PWD/$PREFIX"
fi

case "$PREFIX" in
    */) PREFIX="${PREFIX%/}" ;;
esac
[ -n "$PREFIX" ] || die "prefix must be a real directory, not the filesystem root (/)"

if [ "$PREFIX_SET" -eq 1 ] && [ "$(id -u)" -eq 0 ] && [ -n "${SUDO_USER:-}" ]; then
    echo "Note: running as root via sudo with an explicit prefix; system-wide install." >&2
fi

BIN_DIR="$PREFIX/bin"
DEST="$BIN_DIR/tui-explorer"

cd "$SCRIPT_DIR"

command -v cargo >/dev/null 2>&1 \
    || die "cargo not found. Install a Rust toolchain (see README.md 'Installation')."
command -v install >/dev/null 2>&1 || die "install(1) not found; coreutils is required."

echo "Building tui-explorer (release) from $SCRIPT_DIR"
cargo build --release --locked --bin tui-explorer

BIN_SRC="$SCRIPT_DIR/target/release/tui-explorer"
[ -f "$BIN_SRC" ] || die "build did not produce $BIN_SRC"
[ -x "$BIN_SRC" ] || die "build produced $BIN_SRC but it is not executable"

# Refuse up front rather than half install into an unwritable directory.
if [ -e "$DEST" ]; then
    if [ ! -w "$DEST" ]; then
        die "$DEST exists and is not writable; re-run with permission to overwrite it or choose another --prefix"
    fi
else
    probe="$BIN_DIR/.tui-explorer-write-probe.$$"
    if ! (umask 022 && mkdir -p "$BIN_DIR" && touch "$probe" && rm -f "$probe") 2>/dev/null; then
        die "cannot write to $BIN_DIR; re-run with permission to install there, or choose another --prefix"
    fi
fi

if [ -e "$DEST" ]; then
    echo "Existing install found at $DEST, replacing it."
    install -Dm755 "$BIN_SRC" "$DEST"
    echo "Upgraded tui-explorer at $DEST"
else
    install -Dm755 "$BIN_SRC" "$DEST"
    echo "Installed tui-explorer to $DEST"
fi

[ -x "$DEST" ] || die "installed file $DEST is not executable"

if ! version_output="$("$DEST" --version 2>&1)"; then
    echo "$version_output" >&2
    die "installed binary failed its check ('$DEST --version')"
fi

if ! command -v mpv >/dev/null 2>&1; then
    echo "Note: mpv is not installed. Direct video playback requires mpv with a Kitty graphics terminal."
    echo "Audio playback and all other features work without it."
fi

echo "Verified: $version_output"

if path_has_dir "$BIN_DIR"; then
    echo "PATH: $BIN_DIR is on your PATH."
else
    echo "PATH: $BIN_DIR is not on your PATH, so 'tui-explorer' will not be found by name."
    echo "  Add it for the current shell: export PATH=\"$BIN_DIR:\$PATH\""
    echo "  Make it permanent:     echo 'export PATH=\"$BIN_DIR:\$PATH\"' >> ~/.bashrc"
fi
