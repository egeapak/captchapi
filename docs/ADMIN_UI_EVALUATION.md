# Admin UI: options, measurements, recommendation

Evaluation of adding a web console for changing the reloadable configuration at runtime,
with every size claim measured rather than estimated.

## What the UI actually has to do

`src/config/params.rs` marks exactly five parameters `Reload::Live`:

| field | default |
|-------|---------|
| `default_session_ttl_seconds` | 300 |
| `max_session_ttl_seconds` | 3600 |
| `max_validation_attempts` | 3 |
| `captcha_compression` | 40 |
| `cleanup_interval_seconds` | 60 |

Everything else is captured at boot by the listener, the pool, the middleware or the rate
limiter, and `test_only_per_request_values_are_live` pins that list. So the console is
**five editable integers**, plus sixteen read-only rows for context, plus three buttons
(apply / reload / cleanup).

The API it drives already existed; `source`, `editable` and `description` were added to it
alongside the console:

- `GET /api/v1/admin/config` → `{config: {field: {value, reloadable, secret, source,
  editable, description}}, overrides: []}`
- `PATCH /api/v1/admin/config` → same shape back; rejects boot fields, unknown keys, and
  fields pinned by the command line or environment
- `POST /api/v1/admin/config/reload` → adds `ignored` (boot-only drift) and `message`
- `POST /api/v1/admin/cleanup`

All four sit behind `MasterKeyMiddleware`. `ADMIN_CONFIG_WRITE=false` already disables the
PATCH path alone, leaving GET and reload working — the console inherits that for free.

That is a small, static, single-screen form. It is worth being explicit about that before
picking a tool, because most of the candidates below are priced for a much larger app.

## Measurements

Method: all Rust figures are the real `[profile.release]` from `Cargo.toml`
(`lto = true`, `codegen-units = 1`, `opt-level = "z"`, `strip = true`, `panic = "abort"`),
target `x86_64-unknown-linux-gnu`, built with `--features otel` as `release.yml` does.
Frontend figures are production builds (Vite 8) of the *same* console — same fields, same
three actions — written once per framework.

### Rust side

| build | bytes | vs baseline |
|-------|-------:|------------:|
| baseline `captchapi` (rustc 1.94.1) | 3,930,328 | — |
| + provenance and pinning (core, no console) | 3,949,720 | +19,392 (+0.49%) |
| + the SQLite config store and restart (core, no console) | 4,105,656 | +175,328 (+4.46%) |
| **+ hand-written console** (`--features admin-ui`) | **4,133,272** | **+202,944 (+5.16%)** |
| baseline, rustc 1.95 (control) | 3,916,664 | −13,664 |
| **+ Topcoat page**, rustc 1.95 | **5,750,808** | **+1,834,144 (+46.8%)** |

The Topcoat row is measured against the 1.95 control, not the 1.94 baseline, so the
compiler bump is not counted against it.

Most of that total is not the console. Persisting configuration, the generation bookkeeping
that rolls a bad one back, and restarting in place account for 175,328 bytes in the core, paid
by every build; the console's own share is 27,616. The breakdown below predates that work and
describes the console alone.

Two thirds of the console's original figure was not the console either. Tracking which layer supplied each value, so the
API can refuse to change one the command line or environment pinned, costs 19,392 bytes in the
core and is paid by every build whether or not the console is compiled in. The console itself
accounts for the remaining 23,392: 15,040 of HTML and JS, ~2,400 of per-parameter descriptions
in `PARAMS`, and the rest the Rust that serves the page. Gzipped — the proxy for what a
registry stores and a `docker pull` moves — the whole thing grows the binary by 17,980 bytes.

The assets were 7,952 bytes before a design pass added light/dark theming, a narrow-viewport
layout, and visual separation between live, boot and secret fields. That is the honest price
of the polish: +6,314 bytes of CSS and markup, still an order of magnitude under any
framework option below.

Standalone probes, to separate framework cost from the tokio/hyper floor:

| standalone probe, same profile | bytes |
|--------------------------------|------:|
| axum + tokio, renders the config table | 687,944 |
| Topcoat, renders the same table | 2,152,232 |

### Frontend payloads

What lands in the binary, since assets are embedded:

| stack | raw | gzipped |
|-------|----:|--------:|
| **hand-written, no dependencies** | **19,277** | **6,654** |
| hand-written + water.css | 30,520 | 6,623 |
| preact/compat SPA (Vite) | 21,533 | 8,702 |
| hand-written + pico.css (classless) | 78,892 | 13,380 |
| Svelte 5 SPA (Vite) | 38,810 | 15,187 |
| Alpine.js + pico.css | 120,960 | 28,583 |
| htmx + pico.css | 125,852 | 28,423 |
| React SPA (Vite) | 192,581 | 59,980 |
| React + Tailwind v4 + daisyUI | 223,199 | 66,248 |

The framework rows were measured against the pre-polish page (7,952 bytes of markup), so
each one understates its own total by the same ~6 KB of styling the hand-written row now
carries. The comparison between them is unaffected.

Two results worth keeping. **Preact/compat is a drop-in for React here** — identical
`App.tsx`, a three-line Vite alias — and costs 21.5 KB against React's 192.6 KB, a 9x
reduction for no source change. And **Tailwind + daisyUI is cheap on its own** (30.2 KB of
CSS for a fully styled page, 6.2 KB gzipped, because the JIT only emits classes the page
uses); it is React underneath it that is expensive, not the styling.

Embedding costs the raw column. Pre-compressing at build time and serving with
`Content-Encoding: gzip` would cost the gzipped column instead — irrelevant at 8 KB, but it
would take React+Tailwind from 223 KB to 66 KB if that stack were ever chosen. Note this is
a *binary* size question only: the existing `CompressionLayer` already gzips these responses
on the wire (measured: `app.js` goes out as 1,490 bytes, not 4,278).

## Topcoat

The Topcoat asked about is [tokio-rs/topcoat](https://github.com/tokio-rs/topcoat) v0.4.0,
announced 2026-07-22 — not the archived Adobe CSS library of the same name, whose last npm
publish was 2013. It is a server-rendered Rust framework: components are async Rust, markup
comes from a `view!` macro, and reactivity is HTML snippets plus metadata rather than WASM.
Genuinely nice technology, and the right shape for this kind of page — no client build step,
no bundle to manage.

It does not fit here, for three reasons that are independent of taste:

1. **+1.83 MB, +47% on a binary whose size is a documented feature.** `CLAUDE.md` spends a
   section on recovering 332 KB from the SQLite amalgamation and 105 dependencies from the
   renderer. This spends five times that on one admin screen.
2. **No axum interop.** Its docs expose `start`/`serve` over its own router and describe no
   conversion to an axum `Router` or tower `Service`. So the console means either a second
   HTTP listener in-process (its own port, own middleware, own rate limiting — none of
   captchapi's auth applies) or migrating the whole service off axum. The announcement itself
   frames the two as covering different use cases.
3. **MSRV 1.95 vs this repo's 1.94**, and edition 2024. Not a blocker on its own, but it
   raises the floor for every consumer of the crate and the NAPI bindings.

It also brings **55 crates not already in the tree**, including `tungstenite` (websockets),
`aes-gcm`, `cookie`, `brotli` and `time` — all reachable, none needed by five integer fields.

If captchapi were being written today as a server-rendered app, Topcoat would be worth a
serious look. Bolting it onto an existing axum service for one screen is not what it is for.

## Recommendation

**Hand-write it: static HTML + one `<script>` + `<style>`, embedded with `include_bytes!`,
behind an off-by-default `admin-ui` cargo feature.** That is what is on this branch.

The reasoning is that the framework tax is priced against a problem this page does not have.
Alpine or htmx buy declarative reactivity for 46-51 KB; this page's entire dynamic behaviour
is "render 21 rows, track which of 5 inputs changed, PATCH the diff" — about 130 lines of
DOM calls. React buys a component ecosystem for 193 KB and a Node toolchain in CI; there is
one component. Pico.css buys a full classless stylesheet for 71 KB; the page has a table,
three buttons and one input style, which is 60 lines of CSS.

The feature flag matters for the same reason `otel` is a feature: anyone driving this from
`curl`, a TOML file or `captchapi reload` should not carry the bytes. Off by default keeps
the published image byte-identical to today unless someone opts in.

If the console ever outgrows hand-written DOM — session browsing, key management, charts —
**Preact + Vite** is the upgrade to reach for, not React. Same JSX, same mental model, 21 KB.
Add Tailwind for styling when that happens; it prices well.

### Choices in the prototype

- **Mounted at `/admin`, unauthenticated.** It is the page that *asks* for the master key,
  so it cannot sit behind the master-key middleware. It contains no secrets and no
  configuration; every value it displays comes from `GET /api/v1/admin/config`, which is
  authenticated. Routes are registered at absolute paths rather than nested, because
  `nest("/admin", route("/"))` answers `/admin` but 404s on `/admin/`.
- **The key lives in a JS variable, not `localStorage` or `sessionStorage`.** It dies with
  the tab, and "Lock" clears it. A strict CSP (`default-src 'none'; script-src 'self'`)
  backs that up, since anything that could inject a script could read the variable. A test
  asserts the page ships no third-party script and no CDN reference.
- **`include_bytes!`, not `ServeDir`.** The deployment target is a single static binary on
  distroless or scratch; files on disk would need staging and would add `tower-http/fs`.

### Verifying the page itself

`.playwright/admin-console.mjs` drives the console in a real browser: unlock, edit, apply,
revert, reload, cleanup, lock, both themes, 420px. It exists because nothing else in the suite
runs the page's JavaScript — the Rust tests cover the routes, headers and CSP, and Bruno covers
the API the page calls, but neither catches the page and the server disagreeing. Its assertions
check the server through a separate API call rather than reading the DOM back.

It is not one of the five verification steps in `CLAUDE.md`, because it needs Node and a
browser that the Rust and Bruno suites do not. Run it when changing `assets/admin/*` or the
shape of `GET /api/v1/admin/config`.

### Not done in the prototype

Deliberately, since this is an evaluation branch: no Bruno coverage, no `docs/API.md`
section, and no runtime switch — the console is compile-time on or off, with no
`ADMIN_UI_ENABLED` parameter. If the recommendation is accepted, those three follow, and
`ADMIN_UI_ENABLED` should be a `Reload::Boot` parameter so it appears in `config show`
alongside `admin_config_write`.

## Reproducing

```bash
cargo build --release --features otel                # baseline
cargo build --release --features otel,admin-ui       # with the console
stat -c%s target/release/captchapi
```

Then run the server and open <http://localhost:3000/admin>.
