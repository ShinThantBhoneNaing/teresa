#!/bin/sh
# TERESA client installer (Linux, macOS) — no Rust toolchain needed.
#
#   curl -fsSL __TERESA_BASE__/install.sh | sh
#
# Later updates: `teresa update` (the client also offers them on launch).
set -eu

BASE="__TERESA_BASE__"

case "$(uname -s)" in
Darwin) os=macos ;;
Linux) os=linux ;;
*)
    echo "unsupported OS $(uname -s) — on Windows run: irm $BASE/install.ps1 | iex" >&2
    exit 1
    ;;
esac
case "$(uname -m)" in
arm64 | aarch64) arch=aarch64 ;;
x86_64 | amd64) arch=x86_64 ;;
*)
    echo "unsupported CPU $(uname -m)" >&2
    exit 1
    ;;
esac
platform="$os-$arch"

# Replace an existing teresa (e.g. an old `cargo install`) in place so PATH
# keeps finding it; otherwise ~/.local/bin.
dir="$HOME/.local/bin"
if existing=$(command -v teresa 2>/dev/null) && [ -w "$(dirname "$existing")" ]; then
    dir=$(dirname "$existing")
fi
mkdir -p "$dir"

tmp="$dir/.teresa-download.$$"
headers="$dir/.teresa-headers.$$"
trap 'rm -f "$tmp" "$headers"' EXIT

echo "✦ downloading teresa for $platform from $BASE"
if ! curl -fSL --progress-bar -D "$headers" -o "$tmp" "$BASE/api/v1/client/download/$platform"; then
    echo "no build for $platform on this node yet — ask whoever deploys it" >&2
    exit 1
fi

want=$(tr -d '\r' <"$headers" | awk -F': ' 'tolower($1) == "x-teresa-sha256" { print $2 }' | tail -1)
if command -v sha256sum >/dev/null 2>&1; then
    got=$(sha256sum "$tmp" | cut -d' ' -f1)
else
    got=$(shasum -a 256 "$tmp" | cut -d' ' -f1)
fi
if [ -n "$want" ] && [ "$got" != "$want" ]; then
    echo "checksum mismatch — download corrupted, try again" >&2
    exit 1
fi

chmod 755 "$tmp"
mv -f "$tmp" "$dir/teresa"
echo "✦ installed $dir/teresa"

case ":$PATH:" in
*":$dir:"*) ;;
*)
    echo
    echo "! $dir is not on your PATH. Add it once:"
    echo "    echo 'export PATH=\"$dir:\$PATH\"' >> ~/.$(basename "${SHELL:-sh}")rc"
    ;;
esac
echo "  run: teresa"
