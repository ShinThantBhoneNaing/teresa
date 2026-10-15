#!/bin/sh
# Writes manifest.json for the client builds in a releases directory, which
# teresad serves to `teresa update`. Run by deploy-friday.sh on friday:
#
#   scripts/publish-client.sh <build-stamp> <releases-dir>
#
# Every teresa-<os>-<arch>[.exe] file in the directory is listed under the
# platform key <os>-<arch>. The manifest is swapped in atomically.
set -eu

build="$1"
dir="$2"
cd "$dir"

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

{
    printf '{\n  "build": "%s",\n  "platforms": {' "$build"
    sep=''
    for file in teresa-*; do
        [ -f "$file" ] || continue
        key=${file#teresa-}
        key=${key%.exe}
        printf '%s\n    "%s": {"file": "%s", "sha256": "%s", "size": %s}' \
            "$sep" "$key" "$file" "$(sha256 "$file")" "$(wc -c <"$file" | tr -d ' ')"
        sep=,
    done
    printf '\n  }\n}\n'
} >manifest.json.tmp
mv manifest.json.tmp manifest.json

for file in teresa-*; do
    [ -f "$file" ] && echo "  · ${file#teresa-}"
done
