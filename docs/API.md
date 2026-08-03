# TERESA API reference

Everything lives under `/api/v1/`. Authentication is REST; conversation is
WebSocket. The canonical definition of every type below is
`shared/src/frame.rs` (`teresa-proto`) — this document describes, that file
decides.

## REST

### `POST /api/v1/auth/register`

```json
{ "username": "neo", "password": "…", "display_name": "Neo" }
```

`201` →

```json
{ "token": "kTb0…", "user": { "id": "01J…", "username": "neo",
  "display_name": "Neo", "status": "online", "role": "admin",
  "created_at": 1750000000000 } }
```

- Usernames: 2–32 chars of `a-z 0-9 _ - .`, case-folded, first char
  alphanumeric. Passwords: 8–256 chars, no composition rules.
- The first account ever registered gets `role: "admin"`.
- `403` when the server has `open_registration = false`; `409` when the name
  is taken; `429` with a `retry-after` header when the per-IP budget
  (default 5/min) is spent.

### `POST /api/v1/auth/login`

Same response shape. Wrong password and unknown user are indistinguishable
(`401`), including in timing.

### `POST /api/v1/auth/logout`

`Authorization: Bearer <token>` → `204`. Revokes that session only.

### `GET /api/v1/healthz`, `GET /api/v1/metrics`

Liveness and a JSON gauge snapshot. No authentication (they leak counts, not
content; front them with your proxy if that matters to you).

### Error body

Every REST failure:

```json
{ "code": "conflict", "message": "username neo already exists" }
```

`code` ∈ `unauthorized · forbidden · session_expired · not_found · conflict ·
invalid · rate_limited · overloaded · internal`.

## WebSocket

```
GET /api/v1/ws
Authorization: Bearer <token>
```

Text frames, JSON, tagged on `"t"`. The server sends `ready` immediately;
after that, both sides talk freely. Sessions slide: any authenticated
activity extends expiry (default 30 idle days).

### Client → server

| `t` | fields | notes |
| --- | --- | --- |
| `send` | `room`, `body`, `nonce` | post a message; `nonce` echoes back in `posted` |
| `emote` | `room`, `body`, `nonce` | `/me` action |
| `typing` | `room`, `active` | throttle yourself; not rate-counted |
| `join` | `name` | join a channel by name |
| `leave` | `room` | leave |
| `create_room` | `name`, `topic?`, `visibility` | `public · private · invite_only` |
| `delete_room` | `room` | owner / moderator only |
| `invite` | `room`, `username` | restricted rooms need room-moderator standing |
| `open_dm` | `username` | idempotent per pair |
| `history` | `room`, `before?`, `limit` | pages backwards; max 200 |
| `members` | `room` | member list |
| `list_rooms` | — | channels visible to you |
| `set_status` | `status`, `text?` | `offline` is not settable |
| `set_display_name` | `display_name` | |
| `mark_read` | `room` | read marker → newest |
| `ping` | `nonce` (u64) | answered with `pong` |

### Server → client

| `t` | meaning |
| --- | --- |
| `ready` | your profile, joined rooms (with unread counts and DM peers), server info — everything for first paint |
| `message` | a message in a room you are in (includes your own, same id as your `posted`) |
| `posted` | ack of your `send`/`emote`, carrying your `nonce` |
| `history` | one page, oldest-first, plus `complete` |
| `room_list`, `room_created`, `room_deleted` | discovery |
| `joined`, `left` | your membership changed (also pushed when someone opens a DM with you) |
| `member_joined`, `member_left`, `members` | room membership |
| `presence` | someone's status changed (global) |
| `typing` | someone else is typing (your own echo is filtered server-side) |
| `profile_updated` | display-name change |
| `invited` | you were invited to a room |
| `notice` | server narration (shutdown warnings, MOTD) |
| `error` | `code` + `message`; `session_expired` means re-authenticate |
| `pong` | your `ping` back |

### Sample exchange

```json
→ {"t":"send","room":"01J8…","body":"ship it 🚀","nonce":"a1"}
← {"t":"posted","message":{"id":"01J9…","room_id":"01J8…","author":{"id":"01J…","username":"neo","display_name":"Neo"},"kind":"chat","body":"ship it 🚀","created_at":1750000000000},"nonce":"a1"}
← {"t":"message","message":{…same id…}}          (the room broadcast; dedupe by id)
```

### Conventions

- **Ids** are ULIDs — lexicographic order is creation order; use the newest
  message id as the `before` cursor.
- **Timestamps** are integer Unix milliseconds, UTC.
- **Unknown frames**: adding variants is backwards-compatible; skip frames you
  do not recognise. The protocol revision in `ready.server.protocol` bumps
  only for breaking changes.
- **Limits**: messages ≤ 4096 chars; 20 posts /10 s per connection (then an
  in-band `rate_limited` error); a client that stops reading long enough to
  lag its broadcast queue is disconnected with `overloaded` and should
  reconnect (history heals the gap).
- **System lines** (joins, leaves, room creation) are `message` frames with
  `"kind":"system"` and no author — they live in history like everything else.

### Writing a bot

A bot is just a client that never draws: register an account, keep the token,
open the socket, react to `message` frames, answer with `send`. The e2e test
suites (`server/tests/e2e.rs`, `client/tests/pipeline.rs`) are working
reference implementations of the full handshake.
