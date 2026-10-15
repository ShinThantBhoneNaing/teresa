#!/usr/bin/env bash
# Deploy (or update) the team node on friday-server.
#
#   scripts/deploy-friday.sh          sync source, build release, install, restart,
#                                     and publish client builds for `teresa update`
#   scripts/deploy-friday.sh status   service status + last log lines
#   scripts/deploy-friday.sh logs     follow the server log
#   scripts/deploy-friday.sh clean    free friday's disk: build output and
#                                     downloaded crate caches (~1 GB); the
#                                     running node is untouched, the next
#                                     deploy just rebuilds from scratch (~6 min)
#
# Chat data (old messages, shared files) is trimmed from inside teresa by an
# admin: /admin, /admin files [days], /admin messages [days].
#
# Runs from any dev machine on the office LAN with `ssh friday-server` set up.
# On friday it lays out:
#   ~/teresa-src/                     source mirror (rsync'd, no .git, no target)
#   ~/.local/bin/{teresad,teresa}     release binaries
#   ~/.config/teresa/server.toml      server config (written once, then yours)
#   ~/.local/share/teresa/            database + shared files
#   ~/.local/share/teresa/releases/   client builds teammates update from
#   ~/.config/systemd/user/teresad.service
#
# Client builds: Linux is built on friday; macOS (Apple silicon + Intel) on
# this Mac; Windows too when `cargo zigbuild` is installed here
# (brew install zig && cargo install cargo-zigbuild). Teammates then run
# `teresa update` — or get offered the update when teresa starts — with no
# Rust toolchain of their own. New machines install with
#   curl -fsSL http://friday-server.local:7737/install.sh | sh     (Linux, macOS)
#   irm http://friday-server.local:7737/install.ps1 | iex          (Windows)
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"

HOST="${TERESA_HOST:-friday-server}"
PORT="${TERESA_PORT:-7737}"
# Apple targets to build on a Mac; set empty to skip.
MAC_TARGETS="${TERESA_MAC_TARGETS-aarch64-apple-darwin x86_64-apple-darwin}"
# Every deploy stamps a fresh build; clients treat any other stamp as an update.
BUILD="$(date +%Y-%m-%d.%H%M).$(git rev-parse --short HEAD 2>/dev/null || echo nogit)"
git diff --quiet HEAD 2>/dev/null || BUILD="$BUILD-dirty"

remote() { ssh -o ConnectTimeout=8 "$HOST" "$@"; }

case "${1:-deploy}" in
status)
    remote 'systemctl --user --no-pager status teresad | head -12; echo; journalctl --user -u teresad --no-pager -n 15'
    exit 0
    ;;
logs)
    exec ssh -t "$HOST" 'journalctl --user -u teresad -f'
    ;;
clean)
    remote 'usage() { du -sc "$@" 2>/dev/null | tail -1 | cut -f1; }
        cd ~
        targets="teresa-src/target .cargo/registry/cache .cargo/registry/src .cargo/git/checkouts"
        before=$(usage $targets)
        rm -rf $targets
        after=$(usage $targets)
        echo "✦ freed $(( (before - after) / 1024 )) MiB of build caches on friday"
        df -h / | tail -1 | awk "{print \"✦ disk: \" \$3 \" used, \" \$4 \" free\"}"
        systemctl --user is-active teresad >/dev/null && echo "✦ teresad still running"'
    exit 0
    ;;
deploy) ;;
*)
    echo "usage: $0 [deploy|status|logs|clean]" >&2
    exit 2
    ;;
esac

echo "✦ reaching ${HOST}…"
remote true || {
    echo "cannot reach ${HOST} — are you on the office LAN?" >&2
    exit 1
}

STAGE=target/releases
rm -rf "$STAGE"
mkdir -p "$STAGE"
if [ "$(uname -s)" = Darwin ] && [ -n "$MAC_TARGETS" ]; then
    for target in $MAC_TARGETS; do
        echo "✦ building client for $target here"
        rustup target list --installed | grep -qx "$target" || rustup target add "$target"
        TERESA_BUILD="$BUILD" cargo build --release --locked -p teresa-client --target "$target" 2>&1 | tail -1
        case "$target" in
        aarch64-*) cp "target/$target/release/teresa" "$STAGE/teresa-macos-aarch64" ;;
        x86_64-*) cp "target/$target/release/teresa" "$STAGE/teresa-macos-x86_64" ;;
        esac
    done
fi
if cargo zigbuild --version >/dev/null 2>&1; then
    echo "✦ building client for Windows here"
    rustup target list --installed | grep -qx x86_64-pc-windows-gnu || rustup target add x86_64-pc-windows-gnu
    TERESA_BUILD="$BUILD" cargo zigbuild --release --locked -p teresa-client --target x86_64-pc-windows-gnu 2>&1 | tail -1
    cp target/x86_64-pc-windows-gnu/release/teresa.exe "$STAGE/teresa-windows-x86_64.exe"
else
    echo "  (no Windows client build: brew install zig && cargo install cargo-zigbuild)"
fi

echo "✦ syncing source"
rsync -az --delete \
    --exclude target --exclude .git --exclude '*.db' --exclude '*.db-*' \
    ./ "$HOST:teresa-src/"

echo "✦ building release on ${HOST} (first build takes a few minutes)"
remote 'set -e
    export PATH="$HOME/.cargo/bin:$PATH"
    command -v cargo >/dev/null || {
        curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
        export PATH="$HOME/.cargo/bin:$PATH"
    }
    cd ~/teresa-src
    TERESA_BUILD='"'$BUILD'"' cargo build --release --locked -p teresa-server -p teresa-client 2>&1 | tail -3
    mkdir -p ~/.local/bin
    install -m 755 target/release/teresad target/release/teresa ~/.local/bin/'

echo "✦ publishing client build $BUILD"
# --delete: a platform not rebuilt this time must not be offered under the
# new build stamp with an old binary.
remote 'mkdir -p ~/.local/share/teresa/releases'
rsync -az --delete "$STAGE/" "$HOST:.local/share/teresa/releases/"
remote "set -e
    releases=~/.local/share/teresa/releases
    install -m 755 ~/teresa-src/target/release/teresa \"\$releases/teresa-linux-\$(uname -m)\"
    sh ~/teresa-src/scripts/publish-client.sh '$BUILD' \"\$releases\""

echo "✦ configuring"
remote "set -e
    mkdir -p ~/.config/teresa ~/.local/share/teresa ~/.config/systemd/user
    if [ ! -f ~/.config/teresa/server.toml ]; then
        cat > ~/.config/teresa/server.toml <<TOML
# TERESA team node on friday-server. Edited by hand from here on;
# deploy-friday.sh never overwrites this file.
bind = \"0.0.0.0\"          # the office LAN; no TLS inside it
port = $PORT
name = \"friday\"
motd = \"friday node · internal dev channel · /send <path> to share files\"
database = \"\$HOME/.local/share/teresa/teresa.db\"
files_dir = \"\$HOME/.local/share/teresa/files\"
open_registration = true
# New accounts auto-join the first room.
seed_rooms = [\"sevendev\", \"general\", \"backend\", \"frontend\", \"security\", \"devops\", \"random\"]

[limits]
upload_max_mb = 8192
TOML
    fi
    # The client on friday itself talks to the local node.
    [ -f ~/.config/teresa/config.toml ] || printf 'server = \"127.0.0.1\"\nport = $PORT\n' > ~/.config/teresa/config.toml

    cat > ~/.config/systemd/user/teresad.service <<UNIT
[Unit]
Description=TERESA chat node
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=%h/.local/bin/teresad --config %h/.config/teresa/server.toml
Restart=on-failure
RestartSec=2
# Ctrl-C semantics: the server warns rooms, then drains.
KillSignal=SIGINT
TimeoutStopSec=10

[Install]
WantedBy=default.target
UNIT
    systemctl --user daemon-reload
    systemctl --user enable teresad >/dev/null 2>&1
    systemctl --user restart teresad"

sleep 1
if remote "curl -fsS http://127.0.0.1:$PORT/api/v1/healthz" >/dev/null; then
    echo "✦ teresad is up on ${HOST}:${PORT}"
else
    echo "teresad did not answer — run: $0 status" >&2
    exit 1
fi

if [ "$(remote 'loginctl show-user "$USER" -p Linger --value' 2>/dev/null)" != "yes" ]; then
    echo
    echo "! lingering is off: the node stops when friday's last session ends."
    echo "  fix once with:  ssh -t $HOST 'sudo loginctl enable-linger \$USER'"
fi
