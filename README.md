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
- **File transfer** — `/send <path>` shares a file or a whole folder (zipped
  on the fly, unpacked on arrival) into a channel, or `/send @bob <path>`
  straight to one person. Raw-bytes HTTP streaming in 1 MiB chunks, SHA-256
  verified on every download, with a live transfer dock: throughput, ETA and
  a hex readout of the wire
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

## Install — Linux · macOS · Windows

Everything is built from source with Rust; the result is two dependency-free
binaries. The steps below end with `teresa` callable from any terminal.

### 1. Install Rust (once, any OS)

**Linux / macOS** — in a terminal:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

Accept the defaults, then restart the terminal (or `source "$HOME/.cargo/env"`).

**Windows** — download and run
[rustup-init.exe](https://rustup.rs); accept the defaults. If it asks,
let it install the *Visual Studio C++ Build Tools*. Then open a **new**
PowerShell.

You also need a C compiler for the bundled SQLite:

| OS | What to do |
| --- | --- |
| macOS | `xcode-select --install` (already present if you use Homebrew) |
| Debian/Ubuntu | `sudo apt install build-essential` |
| Fedora | `sudo dnf install gcc` |
| Arch | `sudo pacman -S base-devel` |
| Windows | covered by the Build Tools rustup installs |

### 2. Build and put `teresa` on your PATH (all OSes, same commands)

```sh
git clone https://github.com/ShinThantBhoneNaing/teresa.git
cd teresa
cargo install --path client    # installs `teresa`
cargo install --path server    # installs `teresad` (only needed on the host)
```

`cargo install` places binaries in `~/.cargo/bin` (Windows:
`%USERPROFILE%\.cargo\bin`), which rustup already added to your PATH —
so this *is* the PATH setup. Verify from a new terminal:

```sh
teresa --version    # → teresa 1.0.0
```

If the command is not found, the PATH entry just isn't loaded yet:
restart the terminal, or add it manually — Linux/macOS: append
`export PATH="$HOME/.cargo/bin:$PATH"` to `~/.zshrc` / `~/.bashrc`;
Windows: *Settings → System → About → Advanced system settings →
Environment Variables → Path → New →* `%USERPROFILE%\.cargo\bin`.

### 3. Run it

The team node runs on **friday-server** (the office Linux box), so on the
office LAN every dev just runs:

```sh
teresa            # connects to friday-server.local:7737 by default
```

Hosting your own node instead (defaults: 127.0.0.1:7737, SQLite ./teresa.db):

```sh
teresad                              # on the host
teresa --server <host-address>       # every dev, or set `server` in the config
```

Teammates on Tailscale use the `.ts.net` name.

### Maintaining the friday node

`scripts/deploy-friday.sh` does the whole job from any dev machine on the LAN:
syncs the source to friday, builds release binaries there, installs them to
`~/.local/bin`, writes `~/.config/teresa/server.toml` on first run (bind
`0.0.0.0`, data in `~/.local/share/teresa/`, `#sevendev` as the landing
channel) and restarts the `teresad` systemd user service.

```sh
scripts/deploy-friday.sh           # deploy / update
scripts/deploy-friday.sh status    # service state + recent log
scripts/deploy-friday.sh logs      # follow the log
```

**Keeping friday light.** Two kinds of weight build up, and each has its
own broom:

- *Chat data* — shared files and old history. Log in as an admin and use
  `/admin` (see Commands); destructive ones ask for `--yes` first. Make an
  account admin from the server: `ssh friday-server '~/.local/bin/teresad
  --config ~/.config/teresa/server.toml promote <username>'` (or `remove
  <username>` to delete an account from the server side).
- *Build caches* — each deploy leaves ~1 GB of compiler output and crate
  downloads. `scripts/deploy-friday.sh clean` deletes them; the running node
  is untouched and the next deploy simply rebuilds (~6 min).

The node speaks plain HTTP inside the LAN — fine for an office network, not
for the internet. To keep it running with nobody logged in on friday, enable
lingering once: `ssh -t friday-server 'sudo loginctl enable-linger $USER'`.

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
| `/send [@user] <path>` | share a file or folder here, or with one person by DM; folders travel zipped. Spaces and drag-and-dropped paths work unquoted |
| `/zip [@user] <path>` | same, but compress a single file too |
| `/get [n\|name] [--to <dir>]` | pull a shared file (newest, `#n` from its card, or by name). Asks where to save, prefilled with your default folder — Tab completes, Ctrl+D saves *and* makes it the default; `--to` skips the question. SHA-256 verified; folders unpack |
| `/downloads [dir]` | show or set the default download folder (saved in your config) |
| `/files` | list the files shared in this room |
| `/admin` | *admins:* storage readout — database, messages, shared files |
| `/admin files [days] --yes` | *admins:* delete shared files older than `days` (default 30; `0` = all) |
| `/admin messages [days] --yes` | *admins:* delete history older than `days` (default 90), then compact |
| `/admin compact` | *admins:* give freed database space back to the disk |
| `/admin users` | *admins:* list every account — online, role, joined |
| `/admin remove @user --yes` | *admins:* delete an account: kicked off live, sessions revoked, can't sign in again; their messages stay in history. You can't remove yourself |
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
| `Ctrl+U` · `Esc` | clear the typed line |
| `Ctrl+C` | clear the typed line; on an empty line, quit (asks first) |

## Configuration

Written with defaults on first run, at:

| OS | Path |
| --- | --- |
| Linux / macOS | `~/.config/teresa/config.toml` |
| Windows | `%APPDATA%\teresa\config.toml` |

```toml
server = "friday-server.local"   # the default: the team node
port = 7737
tls = true
theme = "matrix"
notifications = true
bell = true
title_flash = true
typing_indicators = true
clock_24h = true
downloads = "/Users/neo/Downloads/teresa"   # /get's prefilled folder (also /downloads <dir>)
```

Per-run overrides work everywhere and need no file at all:
`teresa --server chat.example.com --port 443 --tls true`.

## Team access from anywhere (Tailscale)

The zero-cost way to let every dev reach your node from any network — home,
office, phone hotspot — without a public server, port forwarding, or ever
re-sending an IP address: put the machines on a [Tailscale](https://tailscale.com)
mesh. Every device gets a stable private address that follows it anywhere,
and all traffic is WireGuard-encrypted end to end.

**On the machine hosting `teresad`** (once):

```sh
brew install tailscale                # or the installer for your OS
sudo brew services start tailscale    # daemon starts at boot from now on
sudo tailscale up --operator=$USER    # sign in / join your tailnet

# bind the server to all interfaces so the tailnet can reach it
TERESA_BIND=0.0.0.0 TERESA_DB=~/.config/teresa/teresa.db teresad

tailscale status                      # note this machine's name, e.g.
                                      # myhost.tail364b12.ts.net
```

**Each dev** (once):

1. Install Tailscale ([tailscale.com/download](https://tailscale.com/download))
   and sign in with a free account.
2. Get access to the host's tailnet — either the host invites you
   (*admin console → Users → Invite*, 3 users free) or shares just the one
   machine with your account (*admin console → Machines → ⋯ → Share*,
   free for any number of devs).
3. Point TERESA at the stable name, once, forever:

   ```toml
   # ~/.config/teresa/config.toml
   server = "myhost.tail364b12.ts.net"
   port = 7737
   ```

4. `teresa` — you're in the grid.

   On Windows, PowerShell instead of the heredoc above:

   ```powershell
   New-Item -ItemType Directory -Force "$env:APPDATA\teresa" | Out-Null
   @"
   server = "myhost.tail364b12.ts.net"
   port = 7737
   "@ | Set-Content "$env:APPDATA\teresa\config.toml"
   ```

   Or skip the file entirely: `teresa --server myhost.tail364b12.ts.net`.

The host's LAN IP can change, the host can move continents: the `.ts.net`
name keeps working. Because Tailscale already encrypts every packet,
running the node itself in plaintext (`tls` unset) is fine on a tailnet;
TERESA's own TLS remains available for exposure to the open internet
(see [`docs/INSTALL.md`](docs/INSTALL.md)).

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
