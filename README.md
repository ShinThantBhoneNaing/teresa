<div align="center">

```
████████╗███████╗██████╗ ███████╗███████╗ █████╗
╚══██╔══╝██╔════╝██╔══██╗██╔════╝██╔════╝██╔══██╗
   ██║   █████╗  ██████╔╝█████╗  ███████╗███████║
   ██║   ██╔══╝  ██╔══██╗██╔══╝  ╚════██║██╔══██║
   ██║   ███████╗██║  ██║███████╗███████║██║  ██║
   ╚═╝   ╚══════╝╚═╝  ╚═╝╚══════╝╚══════╝╚═╝  ╚═╝
```

**✦ TERESA ✦**

**A team chat that lives where you do: the terminal.**

A native terminal-first chat platform for developers, delivering real-time
collaboration with a cinematic hacker aesthetic.

*no system is safe*

100 % Rust · Tokio async end-to-end · TLS · Argon2id · SQLite · zero Electron

</div>

---

```text
 ✦ TERESA ✦  zion │ #developers — api work                                     ● Neo @neo
 CHANNELS            │ 14:22 Alice @alice                          │ ONLINE — 3
 ▸ #developers       │ Finished the API 🚀                         │ ▶ Alice · in the zone
   #general          │                                             │ ◈ Bob
   #security     @2  │ 14:23 Bob @bob                              │ ● Neo
   #random        4  │ Looking at the PR — `hub.rs` is clean.      │
                     │                                             │ OFFLINE — 1
 DIRECT              │ 14:24 Neo @neo                              │ ○ Trinity
 ▸ @trinity          │ Nice. Shipping it.                          │
                     │                                             │
                     │ Alice is typing···                          │
 ctrl+w browse       │                                             │
 LINK:SECURE 1ms                     F1 help · ctrl+w rooms · ctrl+p profile · ctrl+n alerts
╭ #developers ──────────────────────────────────────────────────────────────────────────────╮
│ ❯ deployed the fix :fire:▏                                                                │
╰───────────────────────────────────────────────────────────────────────────────────────────╯
```

*(an actual 120×36 session, reconstructed from the terminal stream)*

## What it is

A private Discord/Slack for people who never leave the terminal: a native Rust
TUI client (`teresa`) speaking JSON-over-WebSocket to a native Rust server
(`teresad`) that you host yourself. Hundreds of users per node, single-digit
milliseconds of latency on a LAN, and an interface tuned to feel like the
terminals in *Who Am I* and every hacker movie that got it right.

- **Rooms** — public, private, invite-only; create, delete, browse
- **Direct messages** — one pair, one room, forever; reopen from either side
- **Presence** — online · busy · coding · reviewing · away, with status text
- **Typing indicators**, unread and mention badges, terminal-bell + title-flash
  notifications
- **Markdown** — bold, italics, `inline code`, fenced blocks with real syntax
  highlighting, lists, quotes, tables, links
- **Actions** — `/shoot @bob` guns someone down and a 💥 flashes on their
  name in the member panel; also `/slap` `/hug` `/highfive` `/hack` `/nuke`
  `/coffee` `/fire` `/rekt` (and freeform `/me`)
- **Emoji** — Unicode passes through; `:rocket:` `:fire:` shortcodes expand
- **Every script** — full Unicode end to end, verified with Myanmar (မြန်မာ),
  Cyrillic and emoji; your terminal font does the drawing, TERESA never
  mangles a cluster
- **10 themes** — matrix · cyberpunk · whoami · fsociety · amber · ghost ·
  dracula · nord · gruvbox · catppuccin, switchable live with `/theme`
- **Cinematic** — animated gradient login, boot sequence, connection sweeps,
  spinners, toasts, smooth ~30 Hz animation, mouse-wheel scrollback
- **Security** — Argon2id password hashing, 256-bit session tokens stored only
  as SHA-256, TLS 1.3 (rustls/ring), per-IP and per-connection rate limits,
  role-based permissions
- **Fast** — first paint in ~10 ms, ~9 MB client RSS, ~8 MB server RSS

## Quickstart

```sh
git clone https://github.com/ShinThantBhoneNaing/teresa.git
cd teresa

# build both binaries (Rust 1.85+)
cargo build --release

# 1. start a server (defaults: 127.0.0.1:7737, SQLite in ./teresa.db)
./target/release/teresad

# 2. in another terminal: run the client
./target/release/teresa
```

First launch, no account — the welcome is fully animated (the logo
materialises through a violet→sky gradient, the ✦ mark spins beside
shimmering status text while the node is genuinely probed, and the
narration types itself out):

```
  ████████╗███████╗██████╗ ███████╗███████╗ █████╗          ← revealed line
  ╚══██╔══╝██╔════╝██╔══██╗██╔════╝██╔════╝██╔══██╗           by line through
     ██║   █████╗  ██████╔╝█████╗  ███████╗███████║           the gradient
     ██║   ██╔══╝  ██╔══██╗██╔══╝  ╚════██║██╔══██║
     ██║   ███████╗██║  ██║███████╗███████║██║  ██║
     ╚═╝   ╚══════╝╚═╝  ╚═╝╚══════╝╚══════╝╚═╝  ╚═╝

  ╭───────────────────────────────────╮
  │  ✦ Welcome to TERESA v1.0.0       │
  │  node 127.0.0.1:7737 · plaintext  │
  ╰───────────────────────────────────╯

  ✔ waking the node                    ← spinner + shimmer while it probes
  no identity found for this node.     ← typed out

  ✦ [c]reate a new identity or [l]og in? › c
  ✦ username › neo
  ✦ password › ••••••••••••••••••••••
  ✦ display name (enter = username) › Neo
  ✔ forging identity                   ← Argon2 happening behind the shimmer

  ✦ identity accepted — entering the grid
```

Animations respect reality: no TTY, `NO_COLOR`, or `TERM=dumb` gets plain,
instantaneous text.

The password is Argon2id-hashed server-side and never stored anywhere on the
client. The session token lands in `~/.config/teresa/session.toml` (mode
`0600`) and survives restarts; `teresa logout` revokes and forgets it.

## Commands

| Command | Effect |
| --- | --- |
| `/join <room>` | enter a channel (also switches if already joined) |
| `/create <room> [--private\|--invite-only] [topic]` | new channel |
| `/delete [room]` | delete a channel you own (mods can too) |
| `/dm <user>` | open the direct line to someone |
| `/invite <user>` | invite into the current room |
| `/leave` | leave the current room |
| `/me <action>` | emote |
| `/shoot <user>` | gunshot lands on their name 💥 — also `/slap` `/hug` `/highfive` `/hack` `/nuke` `/coffee` `/fire` `/rekt` |
| `/nick <name>` | change display name |
| `/status <s> [text]` | online · busy · coding · reviewing · away |
| `/theme [name]` | switch themes; bare lists them |
| `/rooms` · `/users` | list channels · refresh member panel |
| `/whoami` · `/version` · `/ping` | identity · versions · round-trip |
| `/clear` | clear local scrollback |
| `/logout` · `/quit` | end session · leave (session intact) |
| `//text` | send a message that starts with a literal `/` |

## Keys

| Key | Effect |
| --- | --- |
| `Ctrl+W` | room switchboard (fuzzy filter, joins unjoined channels) |
| `Alt+↑/↓`, `Alt+1…9` | cycle rooms, jump by number |
| `PgUp/PgDn`, mouse wheel | scrollback (auto-fetches older history) |
| `Tab` | complete usernames, `@mentions`, `/commands` |
| `↑/↓` | input history (your draft survives) |
| `Ctrl+K` | clear scrollback |
| `Ctrl+R` | reconnect now |
| `Ctrl+P` / `Ctrl+N` | profile · notification log |
| `Ctrl+L` | repaint |
| `Esc` | close overlay / jump to newest |
| `Ctrl+C` | quit (asks first) |

## Configuration

`~/.config/teresa/config.toml` (written with defaults on first run):

```toml
server = "192.168.1.15"
port = 7737
tls = true
theme = "matrix"
notifications = true
bell = true
title_flash = true
typing_indicators = true
clock_24h = true
```

Per-run overrides: `teresa --server chat.example.com --port 443 --tls true`.

Server side, `teresad --config server.toml` — see
[`config/server.example.toml`](config/server.example.toml) for every knob:
bind address, TLS cert/key, rate limits, session lifetime, seed rooms, MOTD,
open/closed registration.

## Architecture

```
teresa (TUI, ratatui) ── WSS/HTTPS ── teresad (axum) ── SQLite (WAL)
        │                                    │
   teresa-proto  ◄── one wire contract ──► teresa-proto
```

Three crates, one workspace. The protocol crate is the only shared dependency,
so client and server cannot drift apart without the compiler noticing. The
server fans out through in-memory broadcast channels — the database is never
on the hot path — and every connection is bounded in memory, so one stalled
client cannot hurt the room. The full design, including the threat model and
every load-bearing decision, is in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

- [Installation guide](docs/INSTALL.md) — production setup, TLS, systemd
- [API reference](docs/API.md) — REST endpoints and every WebSocket frame
- [Developer guide](docs/DEVELOPING.md) — layout, tests, conventions
- [Contributing](docs/CONTRIBUTING.md)

## Testing

~280 tests across four layers: protocol round-trips, service logic over
in-memory SQLite, real-socket end-to-end suites (two clients conversing,
DMs pushed live, rate limits enforced), and TUI snapshot tests that render
every theme into a headless buffer. `scripts/check.sh` runs the lot plus
rustfmt and a pedantic clippy wall.

## Roadmap

The wire protocol and storage schema already reserve space for what's next:
file sharing (`/upload`, `/download` — blob endpoint + attachment frames),
a plugin/bot surface (bot-role sessions over the existing API: git, CI and
issue-tracker bridges post like any other client), and PostgreSQL behind the
existing repository traits for multi-node setups.

## License

MIT — see [LICENSE](LICENSE).

## Contributors
@ShinThantBhoneNaing
@Khit Thit (mytx)
