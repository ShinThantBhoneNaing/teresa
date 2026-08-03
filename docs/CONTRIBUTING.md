# Contributing

Patches welcome — this is a tool for people who live in terminals, built by
the same.

## Ground rules

1. **Green gate.** `scripts/check.sh` must pass: rustfmt, pedantic clippy with
   warnings denied, and the full test suite. CI runs exactly that script, so
   you can know before pushing.
2. **Tests ride along.** A bug fix carries the test that would have caught it;
   a feature carries tests at the layer it lives in (see the testing map in
   [DEVELOPING.md](DEVELOPING.md)). Rendering changes update the snapshot
   tests rather than deleting them.
3. **Protocol changes are deliberate.** Anything touching
   `shared/src/frame.rs` gets a paragraph in the PR describing compatibility:
   additive (old clients skip it) or breaking (bump `PROTOCOL_VERSION`, and
   expect pushback).
4. **No new dependencies without a case.** Every crate in the tree is there on
   purpose; a new one needs to say what it buys and what it weighs. Anything
   that drags in a C toolchain requirement (OpenSSL, oniguruma, aws-lc-rs) is
   an automatic no — cross-platform builds from a bare `cargo build` are a
   feature.
5. **Match the voice.** Comments explain why; names are words; modules stay in
   their layer. If your diff needs a service to know about HTTP or a widget to
   open a socket, the design wants a different shape.

## Workflow

- Branch from `main`; keep PRs to one topic.
- Commit messages: a short imperative summary line, body if the why isn't
  obvious.
- For UI-visible changes, paste a capture (the reconstructed-screen trick in
  `client/tests/tui.rs` makes text screenshots easy) into the PR.
- Security-sensitive reports (auth, session handling, rate limiting): open a
  private channel to the maintainers rather than a public issue first.

## Good first territory

- New themes (`client/src/ui/theme.rs` — one constant, self-testing)
- Emoji shortcodes (`client/src/ui/markdown.rs`)
- Slash-command ergonomics (aliases, argument validation messages)
- Additional snapshot coverage for layouts and overlays
