#!/bin/sh
# Install collider, its desktop app and their start-menu entries (Linux).
#
#   scripts/install.sh               build, then install under /usr/local
#   scripts/install.sh --user        install under ~/.local instead (no root needed)
#   scripts/install.sh --prefix DIR  install under DIR
#   scripts/install.sh --uninstall   remove an install (combine with --user or --prefix)
#
# The build runs as you. Only copying into a directory you cannot write to asks for root:
# with sudo in a terminal, or with a graphical pkexec prompt when there is no terminal.
# DESTDIR stages the install under another root, for packaging.
#
# Installed files, relative to the prefix:
#   bin/collider                                    the command-line tool
#   bin/collider-gui                                the desktop app
#   libexec/collider/collider-menu                  launcher behind the terminal entry
#   share/applications/collider.desktop             "Collider" (the desktop app)
#   share/applications/collider-terminal.desktop    "Collider Terminal" (the launcher)
#   share/icons/hicolor/scalable/apps/collider.svg  their icon
set -eu
umask 022

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd -P)
self=$root/scripts/install.sh
release=$root/target/release
files="bin/collider bin/collider-gui libexec/collider/collider-menu
share/applications/collider.desktop share/applications/collider-terminal.desktop
share/icons/hicolor/scalable/apps/collider.svg"

prefix=/usr/local
action=install
build=1

die() {
    echo "install.sh: $*" >&2
    exit 1
}

usage() {
    cat <<EOF
usage: scripts/install.sh [--user | --prefix DIR] [--uninstall] [--no-build]

  --user        install under ~/.local instead of /usr/local
  --prefix DIR  install under DIR
  --uninstall   remove the files an install put there
  --no-build    use the existing release build instead of running cargo
EOF
}

while [ $# -gt 0 ]; do
    case $1 in
        --user) prefix=$HOME/.local ;;
        --prefix)
            [ $# -ge 2 ] || die "--prefix needs a directory"
            prefix=$2
            shift
            ;;
        --prefix=*) prefix=${1#--prefix=} ;;
        --uninstall) action=uninstall ;;
        --no-build) build=0 ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            echo "install.sh: unknown option $1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done
case $prefix in
    /*) prefix=${prefix%/} ;;
    *) die "--prefix must be an absolute path" ;;
esac
dest=${DESTDIR:-}$prefix

is_root() { [ "$(id -u)" -eq 0 ]; }

# True when every directory the install touches (or its nearest existing parent) is writable.
writable() {
    for f in $files; do
        d=$(dirname -- "$dest/$f")
        while [ ! -d "$d" ]; do d=$(dirname -- "$d"); done
        [ -w "$d" ] || return 1
    done
}

ensure_build() {
    if [ "$build" = 1 ] && ! is_root; then
        command -v cargo >/dev/null 2>&1 || die "cargo not found; install Rust from https://rustup.rs"
        echo "Building collider (release)..."
        (cd "$root" && cargo build --release --locked -p collider -p collider-gui)
    fi
    # Never build as root (it would leave root-owned files in target/), and never install
    # a build that is older than the sources.
    for binary in "$release/collider" "$release/collider-gui"; do
        [ -x "$binary" ] || die "no release build at $binary; run 'cargo build --release' first"
        stale=$(find "$root/crates" "$root/Cargo.toml" "$root/Cargo.lock" -type f \
            -newer "$binary" -print 2>/dev/null | head -n 1)
        [ -z "$stale" ] ||
            die "$binary is older than $stale; run 'cargo build --release' first"
    done
}

# The app's entry is named after its Wayland app id (`collider`), so the desktop can match
# its windows to the entry and show the right icon.
app_entry() {
    cat <<EOF
[Desktop Entry]
Type=Application
Name=Collider
GenericName=Stream Downloader
Comment=Download HLS and MPEG-DASH streams and record live broadcasts
Exec="$prefix/bin/collider-gui"
Icon=collider
Terminal=false
StartupNotify=true
StartupWMClass=collider
SingleMainWindow=true
Categories=Network;FileTransfer;
Keywords=hls;dash;m3u8;mpd;stream;download;record;live;video;
EOF
}

terminal_entry() {
    cat <<EOF
[Desktop Entry]
Type=Application
Name=Collider Terminal
GenericName=Stream Downloader (command line)
Comment=A terminal with the collider command ready, and a prompt to download a pasted link
Exec="$prefix/libexec/collider/collider-menu"
Icon=collider
Terminal=true
Categories=Network;FileTransfer;
Keywords=hls;dash;m3u8;mpd;stream;download;record;live;cli;terminal;
EOF
}

do_install() {
    install -Dm755 "$release/collider" "$dest/bin/collider"
    install -Dm755 "$release/collider-gui" "$dest/bin/collider-gui"
    install -Dm755 "$root/packaging/linux/collider-menu" "$dest/libexec/collider/collider-menu"
    install -Dm644 "$root/packaging/icons/collider.svg" \
        "$dest/share/icons/hicolor/scalable/apps/collider.svg"
    mkdir -p "$dest/share/applications"
    app_entry >"$dest/share/applications/collider.desktop"
    terminal_entry >"$dest/share/applications/collider-terminal.desktop"
    chmod 644 "$dest/share/applications/collider.desktop" \
        "$dest/share/applications/collider-terminal.desktop"
    # A stale GTK icon cache hides new icons, so only refresh one that already exists.
    cache=$dest/share/icons/hicolor/icon-theme.cache
    if [ -f "$cache" ] && command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -q -f -t "$dest/share/icons/hicolor" || true
    fi
    echo "Installed collider to $dest"
}

do_uninstall() {
    for f in $files; do rm -f "$dest/$f"; done
    rmdir "$dest/libexec/collider" 2>/dev/null || true
    echo "Removed collider from $dest"
}

# The start-menu cache belongs to the desktop user, so this runs as them, never as root.
refresh_menu() {
    [ -z "${DESTDIR:-}" ] && ! is_root || return 0
    if command -v kbuildsycoca6 >/dev/null 2>&1; then
        kbuildsycoca6 >/dev/null 2>&1 || true
    fi
    case ":${XDG_DATA_DIRS:-/usr/local/share:/usr/share}:$HOME/.local/share:" in
        *":$prefix/share:"*) ;;
        *) echo "note: $prefix/share is not in XDG_DATA_DIRS, so the menu will not show the entry" ;;
    esac
}

if [ "$action" = install ]; then
    ensure_build
fi

if ! is_root && ! writable; then
    [ -z "${DESTDIR:-}" ] || die "$dest is not writable"
    set -- --prefix "$prefix" --no-build
    [ "$action" = install ] || set -- "$@" --uninstall
    echo "Writing to $prefix needs root."
    if [ ! -t 0 ] && [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ] && command -v pkexec >/dev/null 2>&1; then
        pkexec "$self" "$@"
    else
        sudo -- "$self" "$@"
    fi
else
    "do_$action"
fi
refresh_menu
