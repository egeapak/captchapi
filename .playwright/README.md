# Admin console end-to-end check

`admin-console.mjs` drives the admin console at `/admin` in a real browser and captures
screenshots of every state. It exists because nothing else in the suite runs the page's
JavaScript: the Rust tests in `src/routes/admin_ui.rs` cover the routes, headers and CSP, and
the Bruno collection covers the API the page calls, but neither can catch the page and the
server disagreeing.

Not part of the five verification steps in `CLAUDE.md` — it needs Node and a browser, which
the Rust and Bruno suites do not. Run it when changing `assets/admin/*` or the shape of
`GET /api/v1/admin/config`.

## Running it

Playwright is not a dependency of this repository; install it here, where `.gitignore` already
excludes `node_modules/`:

```bash
cd .playwright
npm install playwright
npx playwright install chromium     # skip if PLAYWRIGHT_CHROMIUM is set
```

Start a server built with the console compiled in:

```bash
cargo build --features admin-ui
API_KEY_SALT=<16+ bytes> MASTER_API_KEY=<16+ bytes> ./target/debug/captchapi
```

Then:

```bash
cd .playwright
MASTER_KEY=<the master key> node admin-console.mjs
```

Exits non-zero if any assertion fails. Screenshots land in `.playwright/shots/`.

| variable | default | |
|---|---|---|
| `MASTER_KEY` | — | required; the key the console asks for |
| `BASE_URL` | `http://127.0.0.1:3000` | |
| `SHOT_DIR` | `.playwright/shots` | |
| `PLAYWRIGHT_CHROMIUM` | unset | explicit browser binary, for images that pre-stage one |

## What it asserts

Beyond rendering: that storing a value through the console reaches the database and can be
removed again, that the Restart button appears only when something is pending, that a wrong key
is refused and leaves the page gated, that Revert leaves the
server untouched, that Apply actually changes it — checked with a separate API call, not by
reading the DOM back — that Reload discards the override, that boot and secret fields cannot be
edited, that every row carries a description, that the three scope colours differ, and that the
page holds up at 420px with no console errors.

It writes to the server it points at, so point it at a disposable one.
