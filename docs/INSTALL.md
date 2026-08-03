# Installing TERESA

Two binaries: `teresad` (the server, one per team) and `teresa` (the
client, one per human). Both are single static-ish executables with no runtime
dependencies — SQLite is compiled in, TLS is rustls, nothing links OpenSSL.

## Prerequisites

- Rust 1.85 or newer (`rustup` recommended: <https://rustup.rs>)
- A C compiler for the bundled SQLite (present by default on macOS/Xcode CLT,
  `build-essential` on Debian/Ubuntu, MSVC Build Tools on Windows)

Linux, macOS and Windows are all first-class: no Unix-only syscalls outside
`#[cfg(unix)]` guards, no aws-lc-rs (so no cmake/NASM), pure-Rust regex in the
highlighter.

## Building

```sh
git clone https://github.com/ShinThantBhoneNaing/teresa.git
cd teresa
cargo build --release
# binaries: target/release/teresad  target/release/teresa
```

Install onto your PATH:

```sh
cargo install --path server   # teresad
cargo install --path client   # teresa
```

## Running a server

### Local / LAN, no TLS

```sh
teresad
```

Defaults: bind `127.0.0.1:7737`, database `./teresa.db`, open registration,
seed rooms `#general #backend #frontend #security #devops #random`. The first
account to register becomes the server admin.

To serve a LAN, create `server.toml`:

```toml
bind = "0.0.0.0"
port = 7737
name = "zion"
motd = "welcome to the real world"
database = "/var/lib/teresa/chat.db"
```

```sh
teresad --config server.toml
```

The server logs a warning when binding a non-loopback address without TLS —
fine behind a TLS-terminating reverse proxy, a bad idea on an open network.

### TLS

Bring a certificate (Let's Encrypt, your internal CA, or
`scripts/dev-certs.sh` for development) and add:

```toml
[tls]
cert = "/etc/teresa/fullchain.pem"
key  = "/etc/teresa/privkey.pem"
```

Clients then set `tls = true` in their config. Self-signed certificates must
be trusted by the OS trust store on each client machine (the client verifies
against system roots).

### Environment overrides

Containers and CI can override the file: `TERESA_BIND`, `TERESA_PORT`,
`TERESA_DB`, `TERESA_LOG`, `TERESA_OPEN_REGISTRATION`,
`TERESA_TLS_CERT`, `TERESA_TLS_KEY`.

### Locking registration

A server on the internet should close its doors once the team is in:

```toml
open_registration = false
```

Existing accounts keep logging in; new registrations are refused.

### systemd

```ini
# /etc/systemd/system/teresad.service
[Unit]
Description=TERESA server
After=network-online.target

[Service]
ExecStart=/usr/local/bin/teresad --config /etc/teresa/server.toml
User=teresa
Restart=on-failure
# Ctrl-C equivalent; connections get a shutdown notice and 5s to drain.
KillSignal=SIGINT

[Install]
WantedBy=multi-user.target
```

### Health and metrics

- `GET /api/v1/healthz` → `{"status":"ok","version":"…"}`
- `GET /api/v1/metrics` → connections, online users, totals, uptime (JSON)

## Running the client

```sh
teresa                      # uses ~/.config/teresa/config.toml
teresa --server 192.168.1.15 --port 7737
teresa logout               # revoke + forget the stored session
```

First run writes a default config and walks you through creating an identity
with the animated welcome. State lives in `~/.config/teresa/`
(`%APPDATA%\teresa` on Windows): `config.toml` plus `session.toml` (the
bearer token, `0600`, never the password).

For the best rendering use a terminal with truecolor (`COLORTERM=truecolor` —
iTerm2, kitty, WezTerm, Alacritty, Windows Terminal, GNOME Terminal). Without
it themes automatically downshift to the xterm-256 palette. A Nerd Font is
*not* required; every glyph used is plain Unicode.

## Upgrading

Schema migrations run automatically at server start (forward-only, embedded in
the binary). Take a copy of the database file first for anything you care
about: `sqlite3 chat.db ".backup backup.db"` or plain `cp` while stopped.
