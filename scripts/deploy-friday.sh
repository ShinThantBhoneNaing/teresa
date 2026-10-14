#!/usr/bin/env bash
# Deploy (or update) the team node on friday-server.
#
#   scripts/deploy-friday.sh          sync source, build release, install, restart
#   scripts/deploy-friday.sh status   service status + last log lines
#   scripts/deploy-friday.sh logs     follow the server log
#
# Runs from any dev machine on the office LAN with `ssh friday-server` set up.
# On friday it lays out:
#   ~/teresa-src/                     source mirror (rsync'd, no .git, no target)
#   ~/.local/bin/{teresad,teresa}     release binaries
#   ~/.config/teresa/server.toml      server config (written once, then yours)
#   ~/.local/share/teresa/            database + shared files
#   ~/.config/systemd/user/teresad.service
set -euo pipefail
cd "$(dirname "$0")/.."

HOST="${TERESA_HOST:-friday-server}"
PORT="${TERESA_PORT:-7737}"

remote() { ssh -o ConnectTimeout=8 "$HOST" "$@"; }

case "${1:-deploy}" in
status)
    remote 'systemctl --user --no-pager status teresad | head -12; echo; journalctl --user -u teresad --no-pager -n 15'
    exit 0
    ;;
logs)
    exec ssh -t "$HOST" 'journalctl --user -u teresad -f'
    ;;
deploy) ;;
*)
    echo "usage: $0 [deploy|status|logs]" >&2
    exit 2
    ;;
esac

echo "✦ reaching ${HOST}…"
remote true || {
    echo "cannot reach ${HOST} — are you on the office LAN?" >&2
    exit 1
}

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
    cargo build --release --locked -p teresa-server -p teresa-client 2>&1 | tail -3
    mkdir -p ~/.local/bin
    install -m 755 target/release/teresad target/release/teresa ~/.local/bin/'

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
