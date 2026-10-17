#!/usr/bin/env bash
# Builds the release binaries and packs each one into publish/<name>-<version>-<os>-<arch>.zip
# (bash equivalent of publish.ps1, meant for Debian / Ubuntu and other Linux hosts; also works on macOS).
#
# Usage:
#   ./publish.sh                                    everything that can be built
#   ./publish.sh --platforms linux-x64,linux-arm64  a subset
#   ./publish.sh --clean --keep-build               empty publish/ first, keep target/<triple> for faster re-runs
#
# Options:
#   -p, --platforms LIST   comma separated: windows-x64,windows-arm64,linux-x64,linux-arm64,macos-x64,macos-arm64
#   -o, --out DIR          output folder (default: publish)
#       --profile NAME     cargo profile (default: release)
#       --clean            empty the output folder first
#       --keep-build       keep the temporary build folders of the cross targets (target/<triple>)
#   -h, --help             this help
#
# How each target is built
#   * Same architecture as this machine (Linux x64 on an x64 PC, ...): plain `cargo build`.
#   * Everything else: `cargo zigbuild` (zig is the cross linker); glibc 2.17 is targeted for Linux, so the binaries
#     run on old distributions too.
#   * macOS targets from a non-Mac need a macOS SDK (Apple licence, not downloadable by this script): extract it and
#     export SDKROOT=/path/to/MacOSX.sdk. On a Mac they build natively.
#   * windows-x64 and windows-arm64 from Linux / macOS use `cargo xwin` (real MSVC targets; it downloads the Windows
#     CRT and SDK, which are covered by Microsoft's licence). The result cannot be run on the build machine.
#
# One-time setup on Debian 13
#   sudo apt install build-essential pkg-config curl git zip pipx
#   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh      # rustup, not the (too old) Debian rustc
#   pipx install ziglang && cargo install cargo-zigbuild              # pipx exposes it as `python-zig`: the script detects that
#   sudo apt install clang lld llvm && cargo install cargo-xwin         # only for the Windows targets
#
# Every package contains a folder <name>-<version>-<os>-<arch> with the executable and the README; the executable keeps
# its execute permission inside the zip.

set -uo pipefail
cd "$(dirname "$(readlink -f "$0")")"

ALL_PLATFORMS="windows-x64,windows-arm64,linux-x64,linux-arm64,macos-x64,macos-arm64"
PLATFORMS="$ALL_PLATFORMS"
OUT_DIR="publish"
PROFILE="release"
CLEAN=0
KEEP_BUILD=0

usage() { sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'; }

while [ $# -gt 0 ]; do
    case "$1" in
        -p | --platforms) PLATFORMS="${2:?missing value for $1}"; shift 2 ;;
        -o | --out) OUT_DIR="${2:?missing value for $1}"; shift 2 ;;
        --profile) PROFILE="${2:?missing value for $1}"; shift 2 ;;
        --clean) CLEAN=1; shift ;;
        --keep-build) KEEP_BUILD=1; shift ;;
        -h | --help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if [ -t 1 ]; then
    CYAN=$'\033[36m'; GREEN=$'\033[32m'; RED=$'\033[31m'; OFF=$'\033[0m'
else
    CYAN=""; GREEN=""; RED=""; OFF=""
fi

# ------------------------------------------------------------------ project information
# First `name =` / `version =` of the [package] section.
manifest_value() {
    awk -v key="$1" '
        /^\[/ { in_package = ($0 == "[package]") }
        in_package && $1 == key && $2 == "=" { gsub(/"/, "", $3); print $3; exit }
    ' Cargo.toml
}
NAME="$(manifest_value name)"
VERSION="$(manifest_value version)"
[ -n "$NAME" ] && [ -n "$VERSION" ] || { echo "Cannot read name/version from Cargo.toml" >&2; exit 2; }
PROFILE_DIR="$PROFILE"
[ "$PROFILE" = "dev" ] && PROFILE_DIR="debug"

HOST_TRIPLE="$(rustc -vV | sed -n 's/^host: //p')"
case "$(uname -s)" in
    Linux) HOST_OS=linux ;;
    Darwin) HOST_OS=macos ;;
    *) HOST_OS=windows ;;
esac

# Sets TRIPLE, OS and EXE for a platform key.
platform_info() {
    case "$1" in
        windows-x64) TRIPLE=x86_64-pc-windows-msvc; OS=windows; EXE=.exe ;;
        windows-arm64) TRIPLE=aarch64-pc-windows-msvc; OS=windows; EXE=.exe ;;
        linux-x64) TRIPLE=x86_64-unknown-linux-gnu; OS=linux; EXE="" ;;
        linux-arm64) TRIPLE=aarch64-unknown-linux-gnu; OS=linux; EXE="" ;;
        macos-x64) TRIPLE=x86_64-apple-darwin; OS=macos; EXE="" ;;
        macos-arm64) TRIPLE=aarch64-apple-darwin; OS=macos; EXE="" ;;
        *) return 1 ;;
    esac
}

# ------------------------------------------------------------------ helpers
have() { command -v "$1" > /dev/null 2>&1; }

install_rust_target() {
    if ! rustup target list --installed | grep -qx "$1"; then
        echo "  rustup target add $1"
        rustup target add "$1" || return 1
    fi
}

# Builds one platform; prints the path of the executable on success. Messages go to stderr.
build_platform() {
    local key="$1"
    platform_info "$key" || return 1
    local triple="$TRIPLE" os="$OS" exe="$EXE" bin_dir
    local native=0
    if [ "$triple" = "$HOST_TRIPLE" ] || { [ "$os" = "macos" ] && [ "$HOST_OS" = "macos" ]; }; then native=1; fi

    if [ "$native" = 1 ]; then
        install_rust_target "$triple" >&2 || { echo "FAIL: rustup target add $triple" >&2; return 1; }
        local args=(build --profile "$PROFILE" --locked)
        [ "$triple" != "$HOST_TRIPLE" ] && args+=(--target "$triple")
        echo "  cargo ${args[*]}" >&2
        cargo "${args[@]}" >&2 || { echo "FAIL: cargo build failed" >&2; return 1; }
    elif [ "$os" = "windows" ]; then
        # MSVC targets from Linux / macOS: cargo-xwin downloads the Windows CRT and SDK (Microsoft licence) and links with lld.
        cargo xwin --help > /dev/null 2>&1 || { echo "FAIL: cargo-xwin is required to cross-compile $triple (cargo install cargo-xwin)" >&2; return 1; }
        have clang && { have lld || have lld-link || have ld.lld; } || { echo "FAIL: clang and lld are required (sudo apt install clang lld llvm)" >&2; return 1; }
        install_rust_target "$triple" >&2 || { echo "FAIL: rustup target add $triple" >&2; return 1; }
        echo "  cargo xwin build --profile $PROFILE --locked --target $triple" >&2
        cargo xwin build --profile "$PROFILE" --locked --target "$triple" >&2 || { echo "FAIL: cargo xwin build failed" >&2; return 1; }
    else
        # zig: a real `zig`, or the `python-zig` wrapper that `pipx install ziglang` puts on the PATH, or the Python module.
        if have zig; then :
        elif have python-zig; then export CARGO_ZIGBUILD_ZIG_PATH="$(command -v python-zig)"
        elif python3 -m ziglang version > /dev/null 2>&1; then :
        else echo "FAIL: zig is required to cross-compile $triple (pipx install ziglang)" >&2; return 1
        fi
        cargo zigbuild --help > /dev/null 2>&1 || { echo "FAIL: cargo-zigbuild is required (cargo install cargo-zigbuild)" >&2; return 1; }
        if [ "$os" = "macos" ] && [ -z "${SDKROOT:-}" ]; then
            echo "FAIL: a macOS SDK is required to cross-compile $triple: export SDKROOT=/path/to/MacOSX.sdk, or build on a Mac" >&2; return 1
        fi
        local zig_triple="$triple"
        if [ "$os" = "linux" ]; then
            zig_triple="$triple.2.17"          # glibc 2.17: runs on old distributions too
        fi
        install_rust_target "$triple" >&2 || { echo "FAIL: rustup target add $triple" >&2; return 1; }
        echo "  cargo zigbuild --profile $PROFILE --locked --target $zig_triple" >&2
        cargo zigbuild --profile "$PROFILE" --locked --target "$zig_triple" >&2 || { echo "FAIL: cargo zigbuild failed" >&2; return 1; }
    fi

    if [ "$triple" = "$HOST_TRIPLE" ]; then bin_dir="target/$PROFILE_DIR"; else bin_dir="target/$triple/$PROFILE_DIR"; fi
    [ -f "$bin_dir/$NAME$exe" ] || { echo "FAIL: build succeeded but $bin_dir/$NAME$exe was not found" >&2; return 1; }
    echo "$bin_dir/$NAME$exe"
}

# package <key> <binary> <exe-suffix> -> writes publish/<root>.zip (the executable keeps its execute permission)
package() {
    local key="$1" bin="$2" exe="$3"
    local root="$NAME-$VERSION-$key"
    local zip_path="$PUBLISH/$root.zip"
    local stage
    stage="$(mktemp -d)" || return 1
    mkdir -p "$stage/$root"
    cp "$bin" "$stage/$root/$NAME$exe" && chmod 755 "$stage/$root/$NAME$exe"
    for extra in README.md LICENSE LICENSE.md LICENSE.txt; do
        [ -f "$extra" ] && cp "$extra" "$stage/$root/"
    done
    rm -f "$zip_path"
    # -X: no extra file attributes; Unix permissions are stored by zip itself.
    (cd "$stage" && zip -qrX "$zip_path" "$root") || { rm -rf "$stage"; return 1; }
    rm -rf "$stage"
}

# ------------------------------------------------------------------ main
have zip || { echo "zip is required (sudo apt install zip)" >&2; exit 2; }
have rustup && have cargo || { echo "rustup/cargo not found (see the setup notes at the top of this script)" >&2; exit 2; }

PUBLISH="$PWD/$OUT_DIR"
[ "$CLEAN" = 1 ] && rm -rf "$PUBLISH"
mkdir -p "$PUBLISH"

declare -a R_PLATFORM R_STATUS R_INFO R_TIME
IFS=',' read -ra KEYS <<< "$PLATFORMS"
for key in "${KEYS[@]}"; do
    if ! platform_info "$key"; then
        echo "unknown platform: $key (expected: $ALL_PLATFORMS)" >&2; exit 2
    fi
    echo
    echo "${CYAN}=== $key ($TRIPLE) ===${OFF}"
    start=$SECONDS
    exe_suffix="$EXE"
    bin="$(build_platform "$key")"
    status=$?
    if [ $status -eq 0 ] && package "$key" "$bin" "$exe_suffix"; then
        size="$(du -k "$PUBLISH/$NAME-$VERSION-$key.zip" | cut -f1)"
        printf '  -> %s/%s-%s-%s.zip (%d.%d MB)\n' "$PUBLISH" "$NAME" "$VERSION" "$key" $((size / 1024)) $(((size % 1024) * 10 / 1024))
        R_STATUS+=("OK"); R_INFO+=("$NAME-$VERSION-$key.zip")
    else
        # build_platform already printed the reason on stderr.
        R_STATUS+=("FAILED"); R_INFO+=("see the message above")
    fi
    R_PLATFORM+=("$key"); R_TIME+=("$((SECONDS - start))s")
done

# Once every build is done (successful or not), empty the temporary build folders of the cross targets.
# target/<host>/<profile> is the normal development build and is left alone (use `cargo clean` to remove it too).
if [ "$KEEP_BUILD" = 0 ]; then
    freed=0
    for key in "${KEYS[@]}"; do
        platform_info "$key"
        for t in "$TRIPLE"; do
            if [ "$t" != "$HOST_TRIPLE" ] && [ -d "target/$t" ]; then
                freed=$((freed + $(du -sk "target/$t" | cut -f1)))
                rm -rf "target/$t"
            fi
        done
    done
    printf '\nTemporary build folders removed (%d.%d GB freed).\n' $((freed / 1048576)) $(((freed % 1048576) * 10 / 1048576))
fi

echo
echo "${CYAN}=== Summary ===${OFF}"
printf '%-14s %-7s %-7s %s\n' Platform Status Time Package
failed=0
for i in "${!R_PLATFORM[@]}"; do
    colour="$GREEN"; [ "${R_STATUS[$i]}" = "FAILED" ] && { colour="$RED"; failed=1; }
    printf '%-14s %s%-7s%s %-7s %s\n' "${R_PLATFORM[$i]}" "$colour" "${R_STATUS[$i]}" "$OFF" "${R_TIME[$i]}" "${R_INFO[$i]}"
done
exit $failed
