# Developing TERESA

## Layout

```
teresa/
├── shared/            teresa-proto — the wire contract (ids, frames, validation)
├── server/            teresad
│   ├── migrations/    embedded SQL migrations (forward-only)
│   └── src/
│       ├── api/       axum router, REST handlers, the WebSocket actor
│       ├── services/  Core: auth, rooms, messages, presence (all policy)
│       ├── repo/      persistence traits + the SQLite adapters
│       ├── security/  Argon2, session tokens, rate limiting
│       ├── hub.rs     in-memory fan-out (broadcast channels)
│       ├── state.rs   composition root product (AppState)
│       └── config.rs  server.toml + TERESA_* env overlay
├── client/            teresa
│   └── src/
│       ├── app/       App state, events, reducer, commands, input line
│       ├── ui/        theme, widgets, overlays, boot, markdown, highlight, anim
│       ├── intro.rs   the animated pre-TUI welcome (gradient, shimmer, ✦)
│       ├── net/       AuthClient (REST) + the reconnecting WS supervisor
│       ├── config.rs  config.toml
│       ├── session.rs token storage (0600)
│       └── main.rs    terminal lifecycle + event pump (thin)
├── config/            example configuration files
├── scripts/           dev.sh · dev-certs.sh · check.sh
└── docs/              this directory
```

## The rules the design leans on

1. **The protocol crate is the contract.** Change a frame in
   `shared/src/frame.rs` and fix both compilers' complaints; never mirror a
   type by hand. New frames go through the round-trip test in the same file.
2. **Services own policy; adapters own SQL; handlers own HTTP.** If you are
   writing a permission check in `api/` or SQL in `services/`, you are in the
   wrong file.
3. **The client reducer is the only writer.** `update(app, event)` mutates
   `App`; side effects leave through `app.effects`. Rendering reads `&App` and
   nothing else. Anything you can express as events + assertions belongs in a
   unit test, not a manual test.
4. **Bounded everything.** Every queue between tasks has a capacity and a
   documented overflow behaviour (drop, disconnect, or backpressure — chosen,
   not defaulted).
5. **No secrets on the wire types.** Wire-facing structs live in
   `teresa-proto`; anything with a hash or token in it lives server-side in
   `repo::*Row` types. Conversions go row → wire, never the reverse.

## Everyday commands

```sh
scripts/dev.sh            # server + client against a throwaway DB
scripts/check.sh          # fmt --check, clippy -D warnings, full test suite
cargo test                # everything
cargo test -p teresa-server --test e2e     # socket-level server suite
cargo test -p teresa-client --test tui     # rendering snapshots
cargo test -p teresa-client --test pipeline # client stack ↔ real server
RUST_LOG=teresa_server=debug cargo run -p teresa-server
```

Client logs: set `log_file = "/tmp/teresa.log"` in the client config (the
TUI owns stderr, so logging is file-only by design).

## Testing map

| Layer | Where | Approach |
| --- | --- | --- |
| Protocol | `shared/src/*` | every frame round-trips; wire names pinned |
| Security | `server/src/security/*` | hash/verify properties, token entropy, bucket math with a mock clock |
| Repos | `server/src/repo/sqlite.rs` | every query against in-memory SQLite |
| Services | `server/src/services/*` | permission matrices, fan-out via hub subscriptions |
| API | `server/tests/e2e.rs` | real port, real HTTP, two real WebSocket clients |
| Client core | `client/src/app/*` | reducer driven by synthetic events |
| Rendering | `client/tests/tui.rs` | `TestBackend` buffers, all 7 themes, hostile sizes |
| Full stack | `client/tests/pipeline.rs` | client net layer + reducer against a real server |

A feature is done when the layer it lives in has a test that fails without it.

## Adding things

**A slash command**: parse in `app/commands.rs` (+ its tests) → execute in
`app/update.rs` → add to `COMMAND_NAMES` and the help overlay.

**A protocol frame**: variant in `shared/src/frame.rs` + the round-trip list →
server dispatch arm in `api/ws.rs` → service method → client handling in
`app/update.rs`. The compiler walks you through every site.

**A theme**: one `const` in `client/src/ui/theme.rs`, added to `ROSTER`. The
existing tests pick it up automatically (roster count: update it).

**A storage backend**: implement the four traits in `server/src/repo/mod.rs`,
swap the alias in `server/src/state.rs`. Nothing else moves.

## Style

`rustfmt` formats; pedantic clippy gates (`clippy.toml` holds the doc-word
allowlist). Comments explain *why*, not *what*. Errors are typed at the
boundary they cross. `unsafe` is forbidden in every crate (`#![forbid]`).
