# TERESA — Architecture

> A native terminal team-chat platform. Rust, async end-to-end, no browser anywhere.

---

## 1. System overview

```
                            ┌──────────────────────────────────────┐
                            │             teresad                  │
                            │                                      │
  ┌───────────────┐  WSS    │  ┌────────┐   ┌──────────────────┐   │
  │    teresa     │◄───────►│  │  api   │──►│     services     │   │
  │   (TUI cli)   │  HTTPS  │  │ axum   │   │  auth / rooms /  │   │
  └───────────────┘         │  └───┬────┘   │ messages / pres. │   │
                            │      │        └────────┬─────────┘   │
  ┌───────────────┐         │  ┌───▼────┐            │             │
  │    teresa     │◄───────►│  │  hub   │◄───────────┘             │
  │   (TUI cli)   │         │  │ fanout │                          │
  └───────────────┘         │  └────────┘   ┌────────▼─────────┐   │
                            │               │   repositories   │   │
  ┌───────────────┐         │               │     (traits)     │   │
  │    teresa     │◄───────►│               └────────┬─────────┘   │
  │   (TUI cli)   │         │                        │             │
  └───────────────┘         │               ┌────────▼─────────┐   │
                            │               │  SQLite (sqlx)   │   │
                            └───────────────┴──────────────────┴───┘
                                                     │
                                             (swappable → Postgres)
```

Three crates in one Cargo workspace:

| Crate            | Binary    | Responsibility                                         |
| ---------------- | --------- | ------------------------------------------------------ |
| `teresa-proto`   | —         | Wire protocol, shared domain types, validation, errors |
| `teresa-server`  | `teresad` | Auth, persistence, rooms, fan-out, REST + WebSocket    |
| `teresa-client`  | `teresa`  | Terminal UI, keybindings, rendering, local session     |

`teresa-proto` is the only thing both sides depend on. Neither binary crate
depends on the other, so the wire protocol is the single contract between
them — it cannot drift.

---

## 2. Design principles

1. **The protocol is the contract.** Every frame is a variant of two enums
   (`ClientFrame`, `ServerFrame`) defined once in `teresa-proto`. Adding a
   feature means adding a variant; the compiler then walks you through every
   place that must handle it.
2. **Ports and adapters.** Services depend on repository *traits*, never on
   SQLite. The Postgres backend later is a new adapter, not a rewrite.
3. **Constructor injection, no service locator.** `AppState` is assembled once
   in `main` and handed down. Tests build the same services over in-memory
   SQLite.
4. **The database is not the hot path.** Message fan-out goes through an
   in-memory hub (`tokio::sync::broadcast`); persistence happens on the side.
   A slow disk never stalls the chat.
5. **The UI never blocks.** The TUI is a single-threaded render loop fed by an
   mpsc channel. Network I/O, disk I/O, and syntax highlighting all happen off
   the render thread.
6. **Fail loud in dev, degrade gracefully in prod.** Every fallible boundary
   returns a typed error; the client renders errors as in-chat system lines
   rather than panicking.

---

## 3. Transport

### Handshake

Authentication happens over **HTTPS**, not over the socket. This keeps the
login path rate-limitable and the socket path stateless-until-authorised.

```
1.  POST /api/v1/auth/register   {username, password, display_name}  ->  {token, user}
    POST /api/v1/auth/login      {username, password}                ->  {token, user}
2.  GET  /api/v1/ws              Authorization: Bearer <token>       ->  101 Switching Protocols
3.  <-- ServerFrame::Ready { user, rooms, ... }
```

The session token is a 32-byte CSPRNG value, sent to the client as base64url.
Only its SHA-256 hash is stored server-side, so a database leak does not
yield usable sessions.

### Framing

JSON text frames. Serde's internal tagging on field `t` keeps frames readable
in `tcpdump` and trivially debuggable, while staying compact:

```json
{"t":"msg","room":"01J...","id":"01J...","author":{"id":"01J...","name":"neo"},"body":"ship it","ts":1750000000}
```

JSON was chosen over a binary codec deliberately: at chat volumes (hundreds
of users, single-digit KB/s aggregate) serialisation is not the bottleneck,
and a human-readable protocol is worth far more for a tool developers are
expected to hack on. The `ClientFrame`/`ServerFrame` split means swapping in
MessagePack later is a codec change in two functions.

### Keepalive

Server sends a WebSocket Ping every 30s; a connection that misses two
consecutive pongs is dropped. The client independently sends
`ClientFrame::Ping` for RTT measurement (`/ping`).

---

## 4. Server internals

```
server/src/
├── main.rs              process bootstrap, signal handling
├── config.rs            ServerConfig  (toml + env overlay)
├── error.rs             ServerError -> HTTP status / ServerFrame::Error
├── state.rs             AppState: the composition root
├── api/
│   ├── mod.rs           router assembly, middleware stack
│   ├── auth.rs          /auth/register /auth/login /auth/logout
│   ├── health.rs        /healthz /metrics
│   └── ws.rs            upgrade handler + per-connection actor
├── services/
│   ├── auth.rs          hashing, sessions, permissions
│   ├── rooms.rs         membership, visibility, invites
│   ├── messages.rs      validation, persistence, fan-out
│   └── presence.rs      status, typing, connection counts
├── repo/
│   ├── mod.rs           trait UserRepo, RoomRepo, MessageRepo, SessionRepo
│   └── sqlite.rs        the SQLite adapters
├── hub.rs               room -> broadcast::Sender registry, per-user routing
└── security/
    ├── password.rs      Argon2id
    ├── token.rs         session token mint/verify
    └── ratelimit.rs     token-bucket governor, per-IP and per-user
```

### The connection actor

Each WebSocket becomes two tasks and one channel:

```
        ┌──────────── reader task ────────────┐
socket ─┤  parse ClientFrame -> dispatch to   │──► services ──► hub.publish()
        │  services (auth'd, rate limited)    │
        └─────────────────────────────────────┘

        ┌──────────── writer task ────────────┐
socket ◄┤  merge: hub subscriptions + direct  │◄── mpsc from services
        │  mpsc + keepalive timer             │
        └─────────────────────────────────────┘
```

Splitting read from write means a client that stops reading cannot block the
room's broadcast. The writer's `mpsc` is bounded; if a client falls too far
behind it is disconnected rather than allowed to consume unbounded server
memory. Targeted pushes (invites, DMs opened from the far side) arrive
through a *control inbox* that passes through the reader task, because some
of them change what the actor must be subscribed to.

### Fan-out

`Hub` holds `DashMap<RoomId, broadcast::Sender<Arc<ServerFrame>>>`. Publishing
is O(1) and lock-free per room; subscribers receive an `Arc`, so a 500-member
room serialises the frame once, not 500 times. Presence and DM routing use a
second map, `DashMap<UserId, Vec<ConnId>>`, because a user may be connected
from several terminals at once.

### Capacity

The target of "hundreds of users" is met comfortably: each connection costs
two tasks (~64 KB stack-equivalent in async frames), one bounded channel of
256 frames, and a broadcast receiver per joined room. 500 connections ≈ 40 MB
RSS plus SQLite page cache.

---

## 5. Client internals

```
client/src/
├── main.rs              CLI, first-run identity flow, terminal lifecycle,
│                        event pump, effect execution (bell, title flash)
├── config.rs            ClientConfig (config.toml)
├── session.rs           token storage at ~/.config/teresa, mode 0600
├── intro.rs             the animated pre-TUI welcome (gradient, shimmer, ✦)
├── net/
│   ├── http.rs          AuthClient: register / login / logout over REST
│   └── ws.rs            connection supervisor: dial, pump, reconnect backoff
├── app/
│   ├── mod.rs           App: all mutable UI state + the Effect queue
│   ├── event.rs         Event: Key | Paste | Mouse | Resize | Tick | Net
│   ├── update.rs        the reducer — the only place App mutates
│   ├── commands.rs      slash-command grammar (pure parser)
│   └── history.rs       input line: editing, history ring, Tab completion
└── ui/
    ├── mod.rs           frame layout + responsive column collapse
    ├── theme.rs         the 7 themes + xterm-256 downconversion
    ├── widgets.rs       header, rooms, messages, members, status, input, toasts
    ├── overlays.rs      help, profile, notifications, switcher, dialogs
    ├── boot.rs          boot splash + connection veil
    ├── markdown.rs      pulldown-cmark → styled lines, emoji shortcodes
    ├── highlight.rs     syntect (lazy), pure-Rust regex engine
    └── anim.rs          spinners, progress bars, staged reveals (pure fns)
```

### The loop

Strictly Elm-shaped, because a chat TUI has too much concurrent state to
manage otherwise:

```rust
loop {
    let event = events.recv().await;     // key, network frame, tick, resize
    update(&mut app, event)?;            // pure-ish state transition
    if app.dirty { terminal.draw(|f| ui::render(f, &app))?; }
}
```

Rendering is *pull*: `update` sets a dirty flag, and a ~30 Hz tick coalesces
bursts. A room receiving 200 messages/second still redraws at the tick rate,
not 200 times.

### Startup budget

The <100 ms target is met by doing almost nothing before first paint:

| Step                               | Cost                      |
| ---------------------------------- | ------------------------- |
| binary load + config parse         | ~8 ms                     |
| terminal raw mode + alt screen     | ~2 ms                     |
| first frame drawn (boot animation) | ~5 ms                     |
| network connect                    | async, after first paint  |
| syntect syntax set load            | lazy, on first code block |

The connection is established *behind* the boot animation, so the animation
is honest progress rather than a fake delay. (Measured on the reference
machine: 10 ms to first paint, ~9 MB RSS.)

---

## 6. Data model

```sql
users(id, username UNIQUE, display_name, password_hash, status, role, created_at, last_seen_at)
sessions(token_hash PK, user_id, created_at, expires_at, last_used_at)
rooms(id, name UNIQUE, topic, visibility, kind, owner_id, created_at)
room_members(room_id, user_id, role, joined_at, last_read_id)   -- PK (room_id, user_id)
room_invites(room_id, user_id, invited_by, created_at)          -- PK (room_id, user_id)
messages(id, room_id, user_id, author_username, author_display_name, kind, body, created_at)
```

- `id` columns are ULIDs, minted monotonically per process: sortable by
  creation time even within one millisecond, so `ORDER BY id DESC LIMIT 50`
  is the history query and no separate index on `created_at` is needed.
- `rooms.kind` is `channel | dm`. A DM room is a normal room with two members
  and a canonical name `dm-<lo>-<hi>` (ids sorted), which makes "open the DM
  with alice" an idempotent upsert instead of a search.
- `rooms.visibility` is `public | private | invite_only`.
- `messages` carries a denormalised author stamp (no foreign key): history
  must survive account deletion, and the history query stays joinless.

WAL mode is enabled; readers never block the writer.

---

## 7. Security model

| Concern            | Mechanism                                                                     |
| ------------------ | ----------------------------------------------------------------------------- |
| Password storage   | Argon2id, 19 MiB / 2 passes / 1 lane, per-password salt                        |
| Password transport | TLS 1.3 (rustls/ring); plaintext HTTP only for `bind = 127.0.0.1`              |
| Sessions           | 256-bit CSPRNG token, SHA-256 at rest, 30-day sliding expiry                   |
| Login abuse        | Token bucket per IP (5/min), plus a decoy-hash verify so unknown users cost the same time as wrong passwords |
| Message flood      | Token bucket per connection (20/10 s), then backpressure                       |
| Authorisation      | `Role::{Member,Moderator,Admin}` × room membership, checked in services        |
| Local secrets      | `~/.config/teresa/session.toml`, mode 0600, token only — never the password    |

Explicit non-goal for v1: **end-to-end encryption**. The server sees
plaintext, exactly as Slack does. Claiming E2EE without a vetted ratchet
implementation would be worse than not claiming it. The transport is
encrypted; the threat model is network attackers, not a hostile server
operator. Adding MLS later fits behind the message service without a protocol
break, since `body` is already an opaque string to the fan-out path.

---

## 8. Extension points

**Themes** are data, not code: a `Theme` is a struct of colours resolved at
startup, so adding one is a literal, not a match arm scattered through the
widgets.

**Plugins** (post-1.0) hook the three seams already present:
`ClientFrame` for new commands, `ServerFrame::Notice` for injected messages,
and a `Bot` role whose sessions are minted from an API key. `plugins/git` is
then just a process that holds a bot session and posts to `#dev` — no plugin
API surface inside the binary, which is the only design that survives contact
with real plugins.

**File sharing** reserves `POST /api/v1/blobs` and an attachment frame; the
message body carries an attachment id, so `/upload` and `/download` do not
change the message path.

---

## 9. Testing strategy

| Layer          | Approach                                                          |
| -------------- | ----------------------------------------------------------------- |
| `teresa-proto` | Round-trip property tests: every frame serialises and parses back |
| Services       | Unit tests against in-memory SQLite (`sqlite::memory:`)           |
| API            | Real-socket integration: spawn a server on port 0, drive two real clients |
| TUI            | `ratatui::backend::TestBackend`, assert on the rendered buffer    |
| Full stack     | The client's net layer + reducer against a real server            |

The TUI tests are what make the "cinematic" requirement maintainable: the
boot animation and each theme render into a fixed buffer that is asserted
against, so a refactor that breaks the layout fails CI instead of shipping.
