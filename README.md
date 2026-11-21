# Kourt Yeared

A mock court for friend groups. File a case, send the defendant a summons link, bring in lawyers and a jury, and let the court decide who buys the samosas.

One Rust binary serves the API, the live WebSocket rooms and the front end.

## Run it

```bash
cargo run --release          # http://127.0.0.1:3000
```

| Variable | Default | Purpose |
|---|---|---|
| `KOURT_ADDR` | `127.0.0.1:3000` | Address to listen on |
| `KOURT_DB` | `sqlite://kourt.db` | SQLite file (created on first run) |
| `KOURT_SECURE_COOKIES` | off | Set to `1` when served over HTTPS |

To let friends join, put it behind HTTPS (Caddy or Cloudflare Tunnel are the easy options) and set `KOURT_SECURE_COOKIES=1`.

## How a trial works

1. **Summons.** The plaintiff files the case and shares a summons link. If the defendant doesn't join before the clock runs out, judgment is entered by default and they owe the treat.
2. **Arguments.** Plaintiff, defendant and one lawyer per side make statements. Each side has 3 objections. A side that never speaks loses: a silent defendant is guilty by default, a silent plaintiff has the case dismissed and owes the false-alarm treat.
3. **Vote.** Jurors vote guilty or not guilty (changeable until time is up). Votes stay hidden until the verdict. When every juror has voted the verdict lands immediately.
4. **Verdict.** Guilty: defendant owes the demand. Not guilty: plaintiff owes a false-alarm treat. Tie or empty jury: hung jury, nobody owes.

Trial length is chosen when the case is filed: Quick (12 min), Standard (3 h), Marathon (3 days) or custom (6 min to 14 days). Time is split 25% summons, 50% arguments, 25% vote (minimums apply).

## Layout

- `src/engine.rs` — the case state machine. `decide()` is pure and unit-tested.
- `src/api.rs` — HTTP handlers and the per-viewer snapshot.
- `src/ws.rs` — live rooms. Each socket re-reads its own snapshot on every change.
- `src/auth.rs` — accounts (argon2), cookie sessions, login throttling.
- `web/` — the front end (plain HTML, CSS and JS, embedded into the binary).
- `tests/` — end-to-end suites (see below).

## Tests

```bash
cargo test                                   # state machine
KOURT_ADDR=127.0.0.1:3111 KOURT_DB=sqlite:///tmp/e2e.db cargo run &
python3 tests/api_e2e.py                     # about 70 API checks (the DB path must be /tmp/e2e.db)
```

`tests/ws_live.js` and `tests/ui_flow.js` need `npm i ws jsdom` and the server on port 3111 using `/tmp/ws.db`.

## Security notes

- State-changing requests need an `x-kourt` header (blocks cross-site forms).
- WebSocket handshakes from other origins are refused.
- Sessions are random 256-bit tokens, stored hashed; cookies are HttpOnly and SameSite=Lax.
- Invite links are role-bound; defendant and lawyer links are single-use and claimed atomically.
- All user text is rendered with `textContent`; a strict CSP is set. Referrer policy is `no-referrer`, so invite tokens don't leak to the font host.
