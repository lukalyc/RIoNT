#!/bin/sh
# Install RIONT on macOS or Linux and ensure ~/.local/bin is on PATH.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/lukalyc/RIoNT/master/scripts/install.sh | sh
#
# Optional environment overrides:
#   RIONT_VERSION=0.9.1        Install a specific version (default: latest)
#   RIONT_INSTALL_DIR=...      Install directory (default: "$HOME/.local/bin")
#   RIONT_ARCHIVE_PATH=...     Install from a local .tar.gz instead of downloading
#   RIONT_NO_MODIFY_PATH=1     Do not edit shell rc/profile files

set -eu

REPO_URL="https://github.com/lukalyc/RIoNT"
VERSION="${RIONT_VERSION:-}"
INSTALL_DIR="${RIONT_INSTALL_DIR:-$HOME/.local/bin}"
ARCHIVE_PATH="${RIONT_ARCHIVE_PATH:-}"
NO_MODIFY_PATH="${RIONT_NO_MODIFY_PATH:-0}"

err() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}

detect_target() {
    os=$(uname -s)
    arch=$(uname -m)

    case "$os" in
        Linux)
            case "$arch" in
                x86_64|amd64)
                    printf '%s' "x86_64-linux-gnu"
                    ;;
                *)
                    err "Unsupported Linux architecture '$arch'. Build from source with: cargo install --git $REPO_URL.git"
                    ;;
            esac
            ;;
        Darwin)
            case "$arch" in
                arm64|aarch64)
                    printf '%s' "aarch64-apple-darwin"
                    ;;
                x86_64|amd64)
                    rosetta=$(sysctl -n sysctl.proc_translated 2>/dev/null || printf '0')
                    if [ "$rosetta" = "1" ]; then
                        printf 'Apple Silicon Rosetta detected; selecting native arm64 build.\n' >&2
                        printf '%s' "aarch64-apple-darwin"
                    else
                        printf '%s' "x86_64-apple-darwin"
                    fi
                    ;;
                *)
                    err "Unsupported macOS architecture '$arch'."
                    ;;
            esac
            ;;
        MINGW*|MSYS*|CYGWIN*)
            err "Windows shell detected. Run the PowerShell installer instead: irm https://raw.githubusercontent.com/lukalyc/RIoNT/master/scripts/install.ps1 | iex"
            ;;
        *)
            err "Unsupported OS '$os'."
            ;;
    esac
}

fetch_url() {
    url="$1"
    dest="$2"
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL -A "riont-installer" -o "$dest" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -q -U "riont-installer" -O "$dest" "$url"
    else
        err "Either curl or wget is required to download RIONT."
    fi
}

resolve_latest_version() {
    latest_url="$REPO_URL/releases/latest"
    resolved=""
    if command -v curl >/dev/null 2>&1; then
        resolved=$(curl -fsSLI -o /dev/null -w '%{url_effective}' -A "riont-installer" "$latest_url" || true)
    elif command -v wget >/dev/null 2>&1; then
        resolved=$(wget --max-redirect=5 --spider -S -O /dev/null "$latest_url" 2>&1 \
            | awk '/^[[:space:]]*[Ll]ocation:/{url=$2} END{gsub(/\r/, "", url); print url}' || true)
    else
        err "Either curl or wget is required to download RIONT."
    fi

    ver=$(printf '%s' "$resolved" | sed -n 's|.*/releases/tag/v\{0,1\}\([^/?#]*\).*|\1|p')
    [ -n "$ver" ] || err "Could not determine the latest RIONT release from GitHub. Set RIONT_VERSION (e.g. 0.9.1)."
    printf '%s' "$ver"
}

compute_sha256() {
    file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$file" | awk '{print tolower($1)}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$file" | awk '{print tolower($1)}'
    else
        printf ''
    fi
}

append_line_if_missing() {
    rc_file="$1"
    line="$2"
    mkdir -p "$(dirname "$rc_file")"
    if [ -f "$rc_file" ] && grep -Fq "$line" "$rc_file"; then
        return 1
    fi
    if [ -f "$rc_file" ] && [ -s "$rc_file" ]; then
        # Ensure a trailing newline before appending.
        last_char=$(tail -c 1 "$rc_file" 2>/dev/null || printf '\n')
        [ "$last_char" = "" ] || printf '\n' >> "$rc_file"
    fi
    printf '\n# Added by RIONT installer\n%s\n' "$line" >> "$rc_file"
    return 0
}

add_to_path_profiles() {
    dir="$1"
    os="$2"

    case ":$PATH:" in
        *":$dir:"*)
            return 0
            ;;
    esac

    if [ "$dir" = "$HOME/.local/bin" ]; then
        posix_line='export PATH="$HOME/.local/bin:$PATH"'
        fish_line='fish_add_path -g "$HOME/.local/bin"'
    else
        posix_line="export PATH=\"$dir:\$PATH\""
        fish_line="fish_add_path -g \"$dir\""
    fi

    shell_name=$(basename "${SHELL:-sh}")
    updated=""

    case "$shell_name" in
        zsh)
            if append_line_if_missing "$HOME/.zshrc" "$posix_line"; then
                updated="$HOME/.zshrc"
            fi
            ;;
        bash)
            if append_line_if_missing "$HOME/.bashrc" "$posix_line"; then
                updated="$HOME/.bashrc"
            fi
            # macOS Terminal launches login bash shells that read ~/.bash_profile rather than ~/.bashrc.
            if [ "$os" = "Darwin" ]; then
                if append_line_if_missing "$HOME/.bash_profile" "$posix_line"; then
                    if [ -n "$updated" ]; then
                        updated="$updated and $HOME/.bash_profile"
                    else
                        updated="$HOME/.bash_profile"
                    fi
                fi
            fi
            ;;
        fish)
            if append_line_if_missing "$HOME/.config/fish/config.fish" "$fish_line"; then
                updated="$HOME/.config/fish/config.fish"
            fi
            ;;
        *)
            if append_line_if_missing "$HOME/.profile" "$posix_line"; then
                updated="$HOME/.profile"
            fi
            ;;
    esac

    if [ -n "$updated" ]; then
        printf 'Added %s to PATH in %s.\n' "$dir" "$updated"
        hint_cmd="$posix_line"
        [ "$shell_name" = "fish" ] && hint_cmd="$fish_line"
        printf 'Open a new terminal (or run: %s) to use `riont`.\n' "$hint_cmd"
    else
        printf 'PATH entry for %s already present in your shell config.\n' "$dir"
    fi
}

TARGET=$(detect_target)
OS_NAME=$(uname -s)

TMPDIR_WORK=$(mktemp -d 2>/dev/null || mktemp -d -t riont-install)
trap 'rm -rf "$TMPDIR_WORK"' EXIT INT TERM

if [ -n "$ARCHIVE_PATH" ]; then
    [ -f "$ARCHIVE_PATH" ] || err "Archive not found: $ARCHIVE_PATH"
    ARCHIVE_FILE="$ARCHIVE_PATH"
    printf 'Installing RIONT from local archive: %s\n' "$ARCHIVE_FILE"
else
    if [ -z "$VERSION" ]; then
        printf 'Checking latest RIONT release...\n'
        VERSION=$(resolve_latest_version)
    fi
    VERSION="${VERSION#v}"
    ASSET_NAME="riont-$VERSION-$TARGET.tar.gz"
    ASSET_URL="$REPO_URL/releases/download/v$VERSION/$ASSET_NAME"
    SUMS_URL="$REPO_URL/releases/download/v$VERSION/SHA256SUMS.txt"

    ARCHIVE_FILE="$TMPDIR_WORK/$ASSET_NAME"
    printf 'Downloading %s (v%s)...\n' "$ASSET_NAME" "$VERSION"
    fetch_url "$ASSET_URL" "$ARCHIVE_FILE"

    SUMS_FILE="$TMPDIR_WORK/SHA256SUMS.txt"
    if fetch_url "$SUMS_URL" "$SUMS_FILE" 2>/dev/null; then
        EXPECTED=$(awk -v f="$ASSET_NAME" '$2 == f || $2 == ("*" f) { print tolower($1); exit }' "$SUMS_FILE")
        if [ -n "$EXPECTED" ]; then
            ACTUAL=$(compute_sha256 "$ARCHIVE_FILE")
            if [ -n "$ACTUAL" ] && [ "$ACTUAL" != "$EXPECTED" ]; then
                err "SHA256 checksum mismatch for $ASSET_NAME (expected $EXPECTED, got $ACTUAL)."
            fi
            [ -n "$ACTUAL" ] && printf 'Verified SHA256 checksum.\n'
        fi
    fi
fi

EXTRACT_DIR="$TMPDIR_WORK/extract"
mkdir -p "$EXTRACT_DIR"
tar -xzf "$ARCHIVE_FILE" -C "$EXTRACT_DIR"

BIN_SRC=$(find "$EXTRACT_DIR" -type f -name riont | head -n 1 || true)
[ -n "$BIN_SRC" ] && [ -f "$BIN_SRC" ] || err "riont binary was not found inside '$ARCHIVE_FILE'."

mkdir -p "$INSTALL_DIR"
cp "$BIN_SRC" "$INSTALL_DIR/riont"
chmod 755 "$INSTALL_DIR/riont"

if [ "$OS_NAME" = "Darwin" ]; then
    xattr -d com.apple.quarantine "$INSTALL_DIR/riont" 2>/dev/null || true
fi

if [ "$NO_MODIFY_PATH" != "1" ]; then
    add_to_path_profiles "$INSTALL_DIR" "$OS_NAME"
fi

INSTALLED_VER=$("$INSTALL_DIR/riont" --version)
printf '\nInstalled %s to %s/riont\n' "$INSTALLED_VER" "$INSTALL_DIR"
printf 'Open a new terminal and run: riont\n'
