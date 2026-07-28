# CaptchAPI - Project Documentation

## Overview

CaptchAPI is a REST API service for creating, validating, and consuming CAPTCHA challenges. Built with Rust, it provides a secure, high-performance solution for integrating CAPTCHA verification into applications.

## Documentation

This file provides project overview, architecture, and development workflow. For detailed information:

- **API Documentation**: See `docs/API.md` for complete endpoint specifications, request/response formats, and usage examples

## Architecture

### Technology Stack

- **Language**: Rust (Edition 2021)
- **Web Framework**: Axum 0.8 (async-first, built on Tokio)
- **Database**: SQLite via SQLx 0.9 (async, compile-time checked queries)
- **CAPTCHA Generation**: In-tree renderer (`src/services/captcha/generator.rs`) with in-tree drawing primitives (`drawing.rs`), on `image` with the JPEG feature only
- **Authentication**: API key-based with SHA256 hashing
- **Deployment**: Static musl binary, published multi-arch (amd64 + arm64) in two variants — distroless on the bare tags (7.33 MB unpacked / 2.77 MB pulled) and scratch on `scratch-` prefixed ones (4.43 MB / 2.17 MB)

### Key Features

- ✅ Configurable CAPTCHA generation (difficulty 1-10, dark mode, custom dimensions, custom length)
- ✅ API key authentication for protected operations
- ✅ Public image retrieval (requires session ID)
- ✅ Automatic session expiration and cleanup
- ✅ Validation attempt limiting (max 3 attempts)
- ✅ Case-sensitive solution matching (constant-time, against a stored keyed hash)
- ✅ CAPTCHA solutions stored as HMAC-SHA256, never in plaintext
- ✅ CAPTCHA images encrypted at rest with ChaCha20-Poly1305
- ✅ CAPTCHA solution not returned in API response (removed in v1.0.0 for security)
- ✅ Structured logging with tracing
- ✅ In-process SQLite database (zero external dependencies)
- ✅ Ultra-small Docker image (full LTO and static linking)
- ✅ Zero runtime dependencies (fully static binary)

## Project Structure

```
captchapi/
├── CLAUDE.md                    # This file - project documentation
├── .env.example                 # Example environment configuration
├── Cargo.toml                   # Rust dependencies
├── Cross.toml                   # Cross-compilation config
├── migrations/                  # SQLx database migrations
│   └── 20250101000000_init.sql
└── src/
    ├── main.rs                  # Application entry point (thin wrapper)
    ├── lib.rs                   # Library crate exports
    ├── app.rs                   # App builder (router, services, middleware)
    ├── config/                  # Layered, reloadable configuration
    │   ├── mod.rs               # Config struct, resolution, redacting Debug
    │   ├── params.rs            # PARAMS: the single source of truth for every setting
    │   ├── sources.rs           # CLI / env / stored / env-file / TOML layers and provenance
    │   ├── boot.rs              # Phase two: fold the store in, decide what may be confirmed
    │   └── handle.rs            # ConfigHandle: watch channel, reload, runtime overrides
    ├── error.rs                 # Error types and handling
    ├── restart.rs               # In-place restart (execve), pre-flight bind check
    ├── metrics.rs               # Prometheus-style metrics
    ├── telemetry.rs             # OpenTelemetry tracing setup
    ├── validation.rs            # Centralized input validation
    ├── models/                  # Data structures
    │   ├── mod.rs
    │   ├── session.rs           # Session models
    │   ├── session_config.rs    # Session configuration
    │   └── api_key.rs           # API key models
    ├── services/                # Business logic layer
    │   ├── mod.rs
    │   ├── captcha/             # CAPTCHA generation
    │   │   ├── mod.rs           # CaptchaService (JPEG encoding)
    │   │   ├── generator.rs     # In-tree renderer (vendored from captcha-rs)
    │   │   └── drawing.rs       # In-tree drawing/noise (vendored from imageproc)
    │   ├── auth.rs              # API key hashing
    │   ├── config_store.rs      # SQLite config store and generation bookkeeping
    │   ├── storage.rs           # Database operations
    │   ├── session_ops.rs       # Session orchestration
    │   ├── solution_hash.rs     # Keyed hashing of CAPTCHA solutions
    │   ├── image_cipher.rs      # Encryption of stored CAPTCHA images
    │   ├── hmac.rs              # Shared HMAC-SHA256 primitive
    │   ├── api_key_ops.rs       # API key orchestration
    │   └── rate_limiter.rs      # Rate limiter configuration
    ├── routes/                  # HTTP endpoints
    │   ├── mod.rs
    │   ├── health.rs            # Health check
    │   ├── sessions.rs          # Session CRUD
    │   ├── api_keys.rs          # API key management
    │   └── admin.rs             # Admin operations
    ├── middleware/              # HTTP middleware
    │   ├── mod.rs
    │   ├── auth.rs              # Authentication
    │   ├── metrics.rs           # Request duration tracking
    │   └── request_id.rs        # Request ID injection
    └── tasks/                   # Background tasks
        ├── mod.rs
        ├── cleanup.rs           # Expired session cleanup
        └── log_filter.rs        # Push log_level changes into the installed subscriber
```

## API Documentation

For complete API documentation including all endpoints, request/response formats, and usage examples, see `docs/API.md`

**Quick Reference:**
- **Public**: Health check, Get session details, Get CAPTCHA image (binary JPEG)
- **Protected**: Create sessions, Validate solutions, Delete sessions
- **Admin**: Manage API keys (create, list, update, delete), manual cleanup, show/patch/reload configuration

The API documentation includes:
- Full endpoint specifications
- Request/response examples
- Authentication requirements
- Error codes and handling
- Complete usage flows
- Migration guides

## Configuration

Configuration can come from the command line, the environment, an env file or a TOML file.
Precedence, highest first:

```
admin API > command line > environment > SQLite store > env file (.env) > config file > default
```

The admin overlay is a real layer rather than something merged into the command line, so
`source_of` can tell the two apart. Merging them — which is what the code used to do — made a
field report itself as `cli`-set the moment it was patched once, which under the pinning rule
below would have pinned it against ever being patched again.

**Pinned fields.** A reloadable field whose effective value came from the command line or the
process environment is refused by `PATCH /config` with `409 config_pinned`. Those two layers
are fixed for the life of the process: an override would work until the next reload discarded
it and could never be made durable without a restart, so accepting it would create drift
between the running server and the deployment that declared it. Files are different — they can
be edited and re-read — so an env-file or TOML value stays patchable. `GET /config` reports
`source` and `editable` per field, and the console drives its inputs off `editable` so it can
never offer an edit the API would reject.

**Adding a new setting is two edits:** one row in `PARAMS` (`src/config/params.rs`) and one
field on `Config` (`src/config/mod.rs`). The flag, help text, TOML key, provenance reporting
and the reloadable/boot-only split are all derived from the table. Tests enforce that the two
stay in sync, including that every declared default matches what the code actually produces.

**The SQLite config store.** `config_settings` holds settings that survive a restart, written
through `PUT /api/v1/admin/config/stored`. It sits below the command line and the environment
because those are the recovery path — a stored value that breaks the service must be
overridable with `captchapi --port 3000` without opening SQLite — and above the files, which
are the deployment baseline durable operator intent should override.

**Boot is a two-phase resolve, and has to be.** The store lives in the database, and where that
database is, is itself configured, so nothing can read it until the first pass has opened the
pool. The second pass is a *fresh resolve*, not a reload: at that point nothing has captured a
boot value yet, and a reload would pin the boot fields to the first pass and defeat the point of
storing `server_port`. `config::boot::apply_stored` is that step, extracted from `main.rs` so
its sharpest decision is testable.

**A `Persist` column says what may be stored.** Nine of twenty-three parameters are `Never`, for
exactly three reasons: a secret, needed to open the database the store lives in, or consumed
before the store is read (the `otel_*` trio). It is an allow-list rather than a derived rule,
because two of those reasons are facts about `main.rs`'s ordering no code can infer — so adding
a parameter forces a decision instead of defaulting into storability.

**Generations and rollback.** Every write snapshots the table as a `pending` generation. A boot
that reads it increments `attempts`; a process that serves for 30 seconds marks it `confirmed`;
a boot that finds a `pending` row already attempted restores the newest confirmed snapshot.
`confirm` takes the specific generation the boot reported, never "whatever is pending" — a write
made while the process runs opens a generation it has proved nothing about, and confirming that
would disarm the rollback for the change most likely to need it. A configuration that fails to
*resolve* is skipped to keep the service up, and its generation is deliberately not confirmed:
confirming it would record settings that do not work as the ones every later rollback restores
to.

**Restart is `execve` on this process**, not a supervisor. The PID never changes, so `docker
stop`, Kubernetes, systemd and the PID file keep working with no extra code, where a parent
would inherit PID 1's obligations to reap orphans and forward signals. `POST /admin/restart` is
off by default behind `ADMIN_RESTART_ENABLED`; it dry-runs the configuration and test-binds the
new address first, because "port already in use" is invisible to validation and a restart into
it leaves the service down rather than merely unchanged.

**Recovery needs no shell.** The distroless and scratch images have neither a shell nor
`sqlite3`, so `captchapi config unset <field>`, `captchapi config clear` and
`--ignore-stored-config` (also `IGNORE_STORED_CONFIG`) are the only way to undo a stored value
that prevents startup. `--ignore-stored-config` is checked *before* the generation bookkeeping
runs: a start told to ignore the store must not write to it.

**Reloadable vs boot-only.** Live means the running process can actually adopt a new value:
the per-request and per-tick values — session TTLs, the attempt limit, JPEG compression, the
cleanup interval — *plus* anything holding a handle onto the thing it configures. `log_level`
qualifies because the subscriber is installed behind a `tracing_subscriber::reload::Layer` and
`tasks::log_filter` pushes changes into it. Everything else is captured at startup by the
listener, the connection pool, the middleware or the rate limiter, and a reload reports drift
rather than pretending to apply it.

`with_boot_fields_from` is a hand-written struct literal naming every field, and
`test_with_boot_fields_from_agrees_with_params_on_every_field` walks `PARAMS` to check it stays
in step. Without that test, moving a field between `Boot` and `Live` compiles, passes every
other test, and silently stops applying reloads to one setting.

Reload is triggered by SIGHUP, `captchapi reload`, or `POST /api/v1/admin/config/reload`.

**Three constructors exist for tests**, all resolving against an *empty* process environment so
a developer who happens to export `CAPTCHA_COMPRESSION` cannot change what an unrelated test
sees. `from_static` takes a `Config` and nothing else; `from_static_with_cli` adds command-line
values, for provenance and pinning; `from_static_with` takes a whole `Cli` and a stored layer,
which is the only way to build a server whose *lower* layers are interesting — a stored value
masking an env-file value that no longer resolves, say. Tests must never call
`std::env::set_var`: `cargo llvm-cov` runs the threaded harness, so an exported-and-restored
variable is a data race against every concurrent `env::var` in the same binary.

**Secrets** are file-only on the CLI (`--api-key-salt-file`, `--master-api-key-file`) and cannot
be set in the TOML file at all. `Config` and `Cli` both have hand-written `Debug` impls that
redact them — keep it that way when adding fields.

### CLI

```bash
captchapi                        # run the server (default verb)
captchapi config show            # effective config with provenance, secrets redacted
captchapi config check           # validate and exit (0 ok, 2 bad) — useful in CI
captchapi config unset <FIELD>   # remove one setting from the SQLite store
captchapi config clear           # remove every setting from the SQLite store
captchapi --ignore-stored-config # start without consulting the store at all
captchapi reload                 # signal a running server to re-read its config
captchapi --help
```

### Environment Variables

Create a `.env` file based on `.env.example`:

```bash
# Server
SERVER_HOST=127.0.0.1
SERVER_PORT=3000

# Database
DATABASE_URL=sqlite:./data/captchapi.db
DATABASE_MAX_CONNECTIONS=5

# Security
API_KEY_SALT=CHANGE-THIS-TO-A-RANDOM-SALT-IN-PRODUCTION
MASTER_API_KEY=CHANGE-THIS-TO-A-SECURE-MASTER-KEY-IN-PRODUCTION
# Optional: dedicated key for hashing CAPTCHA solutions (defaults to API_KEY_SALT)
SOLUTION_HASH_SECRET=CHANGE-THIS-TO-A-RANDOM-SECRET-IN-PRODUCTION
# Optional: dedicated key for encrypting stored images (defaults to API_KEY_SALT)
IMAGE_ENCRYPTION_SECRET=CHANGE-THIS-TO-A-RANDOM-SECRET-IN-PRODUCTION

# CAPTCHA Defaults
DEFAULT_SESSION_TTL_SECONDS=300
MAX_SESSION_TTL_SECONDS=3600
MAX_VALIDATION_ATTEMPTS=3
CAPTCHA_COMPRESSION=40

# Rate Limiting
RATE_LIMIT_REQUESTS_PER_SECOND=2
RATE_LIMIT_BURST_SIZE=10
RATE_LIMIT_REVERSE_PROXY=false    # Set true when behind nginx/Cloudflare

# Admin
ADMIN_CONFIG_WRITE=true           # Set false to make the admin API read-only
ADMIN_RESTART_ENABLED=false       # Set true to allow POST /api/v1/admin/restart
IGNORE_STORED_CONFIG=false        # Set true to start without the SQLite config store

# Background Tasks
CLEANUP_INTERVAL_SECONDS=60

# OpenTelemetry (optional; requires a build with --features otel)
OTEL_ENABLED=false
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
OTEL_SERVICE_NAME=captchapi
```

**Important**:
- Always change `API_KEY_SALT` to a random string in production!
- Always change `MASTER_API_KEY` to a strong, random key in production!
- Rotating `SOLUTION_HASH_SECRET` / `IMAGE_ENCRYPTION_SECRET` (or `API_KEY_SALT`, when no dedicated secret is set) invalidates sessions issued before the restart
- The master key has full administrative access - protect it carefully!
- Only enable `RATE_LIMIT_REVERSE_PROXY` if you trust your proxy — clients can spoof headers otherwise

## Database Schema

### Sessions Table
Stores active CAPTCHA sessions with metadata and solutions.

```sql
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,              -- UUID v4
    solution_hash TEXT NOT NULL,      -- HMAC-SHA256(secret, id || solution)
    image_encrypted BLOB NOT NULL,    -- ChaCha20-Poly1305 ciphertext of the JPEG
    created_at INTEGER NOT NULL,      -- Unix timestamp
    expires_at INTEGER NOT NULL,      -- Unix timestamp
    attempt_count INTEGER DEFAULT 0,  -- Failed attempts
    difficulty INTEGER DEFAULT 5,     -- 1-10
    width INTEGER DEFAULT 220,        -- Pixels
    height INTEGER DEFAULT 120,       -- Pixels
    dark_mode INTEGER DEFAULT 0       -- Boolean
);
```

### Config Settings Table
Settings persisted through the admin API, applied as a configuration layer at startup.

```sql
CREATE TABLE config_settings (
    field      TEXT PRIMARY KEY,  -- Config field name, e.g. 'server_port'
    value      TEXT NOT NULL,     -- Raw string, parsed exactly as any other layer
    updated_at INTEGER NOT NULL,
    updated_by TEXT NOT NULL      -- 'admin-api' | 'cli' | 'rollback'
);
```

### Config Generations Table
One snapshot per write, so a configuration that prevents startup rolls back on its own.

```sql
CREATE TABLE config_generations (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at INTEGER NOT NULL,
    snapshot   TEXT NOT NULL,     -- JSON of config_settings at this generation
    status     TEXT NOT NULL,     -- 'pending' | 'confirmed' | 'rolled_back'
    attempts   INTEGER NOT NULL DEFAULT 0
);
```

Pruned on confirmation: everything older than the newest confirmed generation is unreachable,
since a rollback only ever consults the newest row and the newest confirmed one. Pruning on
*confirm* rather than on write is what keeps the last known-good snapshot alive through a burst
of failed attempts.

### API Keys Table
Stores hashed API keys for authentication.

```sql
CREATE TABLE api_keys (
    key_hash TEXT PRIMARY KEY,        -- SHA256(key + salt)
    description TEXT,                 -- Human-readable label
    created_at INTEGER NOT NULL,      -- Unix timestamp
    last_used_at INTEGER,             -- Unix timestamp
    is_active INTEGER DEFAULT 1       -- Boolean
);
```

## Development Workflow

### Setup

```bash
# 1. Clone and navigate to project
cd captchapi

# 2. Copy environment template
cp .env.example .env

# 3. Edit .env and set API_KEY_SALT
nano .env

# 4. Build project (downloads dependencies)
cargo build

# 5. Run migrations (creates database)
cargo run  # Migrations run automatically on startup
```

### Code Quality Standards

**IMPORTANT**: After **EVERY** code change, you **MUST** run these steps **IN ORDER**:

#### Step 1: Format Code
```bash
cargo fmt
```

#### Step 2: Run Linter
```bash
cargo clippy
```

#### Step 3: Check Compilation
```bash
cargo check --all-targets --workspace
```

**Use `--workspace`.** `bindings/nodejs` matches on `AppError` exhaustively with no wildcard
arm, so adding a variant breaks that crate — and a bare `cargo check` will not tell you.

#### Step 4: Run Rust Tests
```bash
cargo nextest run --all-features --workspace
```

**Use `--all-features` here too.** `src/routes/admin_ui.rs` lives behind the `admin-ui`
feature, which is off by default, so a bare `cargo nextest run` silently skips its tests —
including the ones asserting the console ships no third-party script and carries a CSP. CI
runs the suite with `--all-features`; matching it locally is what stops that gap being
discovered on a pull request instead of at your desk.

#### Step 5: Run API Tests
**Note:** Requires server to be running first.

```bash
# Terminal 1: Start server
RATE_LIMIT_REQUESTS_PER_SECOND=100 RATE_LIMIT_BURST_SIZE=200 cargo run

# Terminal 2: Run API tests
./.bruno/Tests/Scripts/test-bruno-full.sh
```

**The default rate limit is too low for this suite.** It fires ~20 requests at the session
endpoints back to back, against a default of 2/s with a burst of 10, so the tail of the run
returns `429` and about seven requests fail. That is the rate limiter working, not a
regression. Start the server with the limits raised, as `ci.yml` does:

```bash
RATE_LIMIT_REQUESTS_PER_SECOND=100 RATE_LIMIT_BURST_SIZE=200 cargo run
```


**All five steps must pass with no errors before committing.**

These steps ensure:
- ✅ Consistent code formatting
- ✅ No common mistakes or anti-patterns
- ✅ Code compiles successfully
- ✅ All unit and integration tests pass
- ✅ HTTP API behaves correctly

### Testing

#### Rust Tests

```bash
cargo nextest run --all-features --workspace   # Run all tests, as CI does
cargo nextest run --lib                        # Unit tests only
cargo nextest run --test sessions_test         # Integration tests
```

#### API Tests (Bruno)

**Prerequisites:** Server must be running.

```bash
# Terminal 1: Start server
cargo run

# Terminal 2: Run tests
./.bruno/Tests/Scripts/test-bruno-full.sh
```

#### Complete Test Workflow

```bash
# Terminal 1: Start server
cargo run

# Terminal 2: Run all tests
cargo nextest run --all-features --workspace
./.bruno/Tests/Scripts/test-bruno-full.sh
```

## Security Considerations

### API Key Security

1. **Storage**: API keys are hashed with SHA256 + salt before storage
2. **Never** log API keys or include them in error messages
3. **Rotate** the `API_KEY_SALT` if compromised (requires re-creating all keys)

### Session Security

1. **Expiration**: Sessions auto-expire based on TTL
2. **Attempt Limiting**: Max 3 validation attempts per session
3. **Auto-Deletion**: Sessions deleted after successful validation
4. **Cleanup**: Background task removes expired sessions every 60s
5. **Solution Hashing**: Only `HMAC-SHA256(secret, session_id || solution)` is stored. The key comes
   from `SOLUTION_HASH_SECRET` (default: `API_KEY_SALT`) and never lives in the database, so reading
   the database does not reveal answers. A plain digest would be useless here — short alphanumeric
   solutions are brute-forced instantly.
6. **Image Encryption**: Images are stored as ChaCha20-Poly1305 ciphertext and decrypted only when
   served — otherwise the stored challenge could simply be OCR'd. Each session uses its own key,
   `HMAC-SHA256(master_key, info || session_id)`, with the master key from `IMAGE_ENCRYPTION_SECRET`
   (default `API_KEY_SALT`). Per-session keys make cross-row reuse and nonce reuse impossible; the
   session ID is also passed as associated data for defence in depth.
7. **No enumerable rendering constants**: glyph, interference and noise colours are drawn from a
   continuous hue, never a fixed palette, and salt-and-pepper specks are no longer pure black and
   white. The renderer is open source, so any fixed set of RGB values is a segmentation key: a
   solver can separate glyphs from background by testing membership in a handful of known colours.
   That is not hypothetical — it is how a vision model attacked these images in testing before
   template-matching against the bundled font. Only lightness and saturation are bounded, and only
   enough to keep glyphs legible. Do not reintroduce a fixed palette for the sake of consistent
   branding. The same rule extends past colour: a letter is no longer described by *one* colour
   either, since `gradient` ramps hue and lightness across it, and `outline` means "filled" is not
   a property a solver can assume. Outline stroke widths are drawn from a continuous range and the
   erosion is subpixel precisely so that they do not collapse onto a handful of enumerable values.
8. **Deformations are per-letter and mutually independent.** Ten of them — jitter, scale, skew,
   wave, rotation, clustering, outline, transparency, gradient, blur — and each is drawn separately
   for every letter, so no single rule describes a whole solution. All but one ramp with difficulty
   from nothing at level 1 to full at level 10, along the concave curve documented below; `blur` is
   pinned flat at every level above 1, also below. The legibility floors are load-bearing and were set by
   measurement, not taste: `MIN_OPACITY` is what survives difficulty-10 gaussian noise, and
   `MIN_OUTLINE_OPACITY` is higher because a hollow letter has an order of magnitude less ink to
   lose. Lowering either, or raising `MAX_OUTLINE_SHARE` to 1.0, trades human solve rate for
   nothing a machine finds harder.

   Measured after the outline/transparency/gradient additions but **before** the concave intensity
   ramp and the post-composite blur, over one grid of 12 challenges (lengths 4-6 x difficulty
   3/5/8/10) served at the shipped JPEG quality and attempted by Haiku 4.5, Sonnet 5 and Opus 5,
   vision only, each told the character set and the exact solution length. Read it as the baseline
   the two later changes were measured against, not as current numbers — difficulty 5 in this table
   is roughly what difficulty 3 renders like now:

   | difficulty | 3 | 5 | 8 | 10 |
   |---|---|---|---|---|
   | vision only, 5 arms | 10/15 | 5/15 | 1/15 | 0/15 |
   | with image tools, 3 arms | 6/9 | 5/9 | 0/9 | 0/9 |

   Difficulty 3 is not a CAPTCHA — every frontier arm cleared it outright. The ramp does its work
   between 5 and 8: **across all eight arms, difficulty 8 and 10 together yielded 1 solve in 48
   attempts.** Haiku 4.5 never solved anything above difficulty 3 in three runs.

   The second row is the one to read carefully, because it is the realistic attacker. Those arms had
   python, Pillow and numpy and were told to crop, upscale, median-filter, split by hue, threshold
   and template-match — Sonnet spent 170 tool calls and 29 minutes, Opus 164 calls and 52 minutes.
   Tooling **raised the ceiling at difficulty 5 and moved nothing at 8 or above.** Tool-equipped Opus
   went 6/6 across difficulty 3 and 5, beating every vision-only arm, and then scored 0/6 at 8 and
   10 like everyone else. It is also the expensive way to attack: ~331k tokens for Opus and ~187k
   for Sonnet against ~46k vision-only, so roughly $0.28 per solve against $0.05 — a tooled attacker
   pays about 6x more per solved CAPTCHA and gains nothing at the difficulty that ships.

   Reproduce with `examples/challenge_set.rs` and `scripts/solve-challenges.py`. Three images per
   cell is a wide error bar: treat the 8-and-above result as "no arm has yet solved one", not as a
   measured zero, and re-measure with more draws before acting on any single cell.

   **To evaluate a single deformation, use the paired A/B pipeline instead** — the grid above cannot
   attribute a change to one deformation, and an unpaired comparison spends most of its statistical
   power on whether one set of random strings happened to be harder than another:

   ```bash
   CAPTCHA_SAMPLE_DIR=/tmp/ab CAPTCHA_AB_FIELD=rotation CAPTCHA_AB_LEVELS=3,5 \
     cargo test --release --lib deformation_ab_set -- --ignored
   python3 scripts/split-ab.py /tmp/ab /tmp/blind      # crossover into two blind arms
   # ... solve /tmp/blind-A and /tmp/blind-B, one JSON file of answers per arm ...
   python3 scripts/score-ab.py /tmp/ab/manifest.json <answers dir>
   ```

   Three things about that pipeline are load-bearing. It renders the *same solution text* under both
   conditions, so per-string difficulty cancels. The crossover split guarantees no solver sees a
   string twice, which would make the second sighting a memory test. And the blind arm directories
   contain images and lengths only — no manifest, no solutions, nothing to read an answer off.

   Read the **flip counts** the scorer prints, not the totals. A deformation that flips as many
   solves on as it flips off is noise however the totals fall, and that is exactly what killed
   `blur`. Choose the difficulty band with headroom: below it every arm solves everything and above
   it every arm solves nothing, so neither end can move whatever you do.

   **`blur` failed three times, and then worked once it was moved after the composite. What it
   mixes with is the entire deformation; how much of it there is barely matters.** The sequence is
   worth keeping, because four measurements of "the same feature" is what located the mechanism:

   | design | result |
   |---|---|
   | unpaired grid, difficulty 3/5/8/10, ramped, pre-composite | 12/36 -> 10/36 solves, 61% -> 55% chars |
   | paired crossover, difficulty 6 and 7, ramped, pre-composite | 11/36 -> 9/36, flips 8 lost / 6 gained |
   | paired crossover, difficulty 3 and 5, flat, pre-composite | 29/36 -> 27/36, flips 5 lost / 3 gained, p = 0.73 |
   | paired crossover, difficulty 3 and 5, flat, **post-composite** | 17/36 -> 12/36, flips **7 lost / 2 gained**, p = 0.18 |

   The first was confounded — the two conditions used different random images, so part of what it
   measured was whether one set of strings happened to be harder. The second fixed that by rendering
   the *same* text under both conditions, and came back symmetric: blur flipped individual solves in
   both directions about equally, which is what noise looks like. The third pinned the intensity flat
   so full blur landed at difficulty 3 and 5 where there was headroom, and difficulty 5 came back
   **identical**, 12/18 both ways.

   Three failures with one thing in common: the blur was applied to the glyph's coverage mask, so it
   softened a letter's edges and then blended a soft letter onto a clean background. The letter
   stayed a distinct object with a fuzzy border, and a fuzzy border is not a segmentation problem.

   Moving the same gaussian to *after* the composite changes what it acts on. It now mixes the letter
   with whatever it overlaps, which under clustering is the neighbouring glyph:

   ```
   difficulty 3, blur off   12/18 solved   chars 81/90 (90%)
   difficulty 3, blur on    12/18 solved   chars 82/90 (91%)     flips 2 / 2
   difficulty 5, blur off    5/18 solved   chars 72/90 (80%)
   difficulty 5, blur on     0/18 solved   chars 47/90 (52%)     flips 5 / 0, p = 0.06
   ```

   **Difficulty 5 went to zero, and all five discordant pairs flipped the same way.** The character
   rate — 90 characters rather than 18 images, so the steadier statistic — fell from 80% to 52%.

   Difficulty 3 not moving is not a disappointment, it is the mechanism confirming itself. Blur is
   pinned flat, so it is at full strength at difficulty 3 too; what is missing there is *clustering*,
   which at that level barely overlaps the letters. With nothing but background to mix into,
   post-composite blur is just the pre-composite version that three measurements found inert. The
   deformation's value is entirely in what it smears the letter into.

   Two consequences worth not forgetting. `MAX_OUTLINE_BLUR` is gone: the old cap existed because a
   sigma near the stroke width closed a hollow letter's counter back up, and blurring composited
   pixels leaves the mask untouched. And it is **expensive** — 36-39% of render time at difficulty
   3-10, against 13-16% for the mask version, because it convolves three channels of canvas per
   letter instead of one channel of a small coverage buffer. Sustained throughput is about 120
   renders/s/core at difficulty 5. That is worth it at a 5/18-to-0/18 effect, but if it needs to come
   down, the honest lever is the one the data points at: it does nothing where letters do not
   overlap, so gating it on clustering would buy back the low-difficulty cost. Measure with
   `CAPTCHA_AB_FIELD=blur cargo test --release --lib deformation_impact -- --ignored --nocapture`.

   **`rotation` is the counter-example, and it is what a deformation that works looks like.** Same
   harness, same two models, same paired crossover, 72 attempts over 18 texts:

   | | rotation off | rotation on |
   |---|---|---|
   | difficulty 3 | 13/18 solved, 92% chars | 14/18 solved, 94% chars |
   | difficulty 5 | 9/18 solved, 83% chars | **4/18 solved, 73% chars** |
   | overall | 22/36, 88% chars | 18/36, 84% chars |

   Difficulty 3 showed nothing, which is expected and is itself a check on the method: rotation
   ramps with difficulty, so there is barely any of it at level 3. Difficulty 5 is where it lands,
   and it lands hard — the solve rate more than halves, the flips run 7 lost against 2 gained
   (p = 0.18), and both models move the same way, Opus 11/18 to 9/18 and Sonnet the same. The
   character rate is the more trustworthy half of that, since it is 90 characters rather than 18
   images: 83% to 73%.

   Treat p = 0.18 as "consistent and worth keeping", not as proof. Eighteen texts is a small sample,
   the two models are correlated so the effective n is nearer 18 than 36, and the single most
   informative cell rests on 9 discordant pairs.

   Rotation is not skew with extra steps, and that distinction is the reason to have it. A shear
   leaves horizontals horizontal, so the crossbar of an `A` and the foot of an `L` stay level and an
   undeformed solution puts every letter on exactly one row.
   `test_rotation_takes_letters_off_a_shared_baseline` asserts that line exists before asserting the
   deformation breaks it. A shared baseline is a free segmentation cue: findable before any glyph is
   read, and worth more to a solver than any individual letter.

   `MAX_ROTATION` is 0.45 radians, about 26 degrees, and the bound is legibility rather than taste.
   Past roughly 30 degrees the reversible pairs start trading places — a rotated `N` reads as `Z`,
   `M` as `W`, `6` as `9` — which costs a human the character outright while costing a solver that
   already knows the character set nothing it cannot brute-force. It costs 6-11% of render time and
   nothing measurable in encoded size.

   **The intensity ramp is concave, not linear, and this is the largest measured change in the
   renderer's history.** `INTENSITY_CURVE` is 0.6, so intensity is `((difficulty - 1) / 9) ^ 0.6`:

   | difficulty | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 |
   |---|---|---|---|---|---|---|---|---|---|---|
   | linear | 0.00 | 0.11 | 0.22 | 0.33 | 0.44 | 0.56 | 0.67 | 0.78 | 0.89 | 1.00 |
   | curve 0.6 | 0.00 | 0.27 | 0.41 | 0.52 | 0.61 | 0.70 | 0.78 | 0.86 | 0.93 | 1.00 |

   The dial was not earning its range. Solve rates ran ~90% at difficulty 3, 50-65% at 5 and ~2% at
   8 and above, so the bottom third of the scale was not a CAPTCHA and the top third was already at
   the floor — about two useful levels out of ten. A concave curve moves intensity into the low band
   where there is solve rate left to take away, and leaves both endpoints alone: difficulty 1 is
   still undeformed and difficulty 10 is still full.

   Paired against the linear ramp at the same difficulty labels:

   ```
   difficulty 3, linear   16/18 solved   chars 88/90 (98%)
   difficulty 3, curved   11/18 solved   chars 82/90 (91%)     flips 5 / 0, p = 0.06
   difficulty 5, linear    4/18 solved   chars 66/90 (73%)
   difficulty 5, curved    0/18 solved   chars 54/90 (60%)     flips 4 / 0, p = 0.13
   overall                20/36 -> 11/36                       flips 9 / 0, p = 0.004
   ```

   **Nine discordant pairs, nine flips, zero reversals.** Nothing else measured here comes close to
   that — every other change has had at least one flip going the other way. It costs 3-7% of render
   time at difficulty 3-5 and nothing at 8 or above, which is simply the cost of the deformations it
   turns up.

   **It changes what an existing difficulty setting means, and that is a breaking behaviour change
   for callers.** A caller pinned at 5 gets images about as hard as the old 7 without changing
   anything, and a caller who chose 2 or 3 for accessibility gets a real step up — measured at 98%
   to 91% character accuracy for frontier models, so the human cost is not nothing. That is the
   intent, but it belongs in release notes. If low difficulty needs to stay genuinely easy, the lever
   is the exponent: 0.8 is a milder version of the same shape.

   **`DEFAULT_DIFFICULTY` came back down to 5 as a consequence of the two changes above**, having
   been raised to 8 when level 5 was still solvable. What level 5 measures on the current renderer,
   pooled over every set solved against it — 84 attempts on 30 distinct images, Opus 5 and Sonnet 5:

   | arm | solved | 95% CI | P(defeat one session in 3 tries) |
   |---|---|---|---|
   | vision only | 4/60 = 6.7% | [2.6%, 15.9%] | [8%, 41%] |
   | with image tools | 3/24 = 12.5% | [4.3%, 31.0%] | [13%, 67%] |
   | **pooled** | **7/84 = 8.3%** | **[4.1%, 16.2%]** | **[12%, 41%]** |

   **Level 5 is a real CAPTCHA against frontier models but it is not a wall, and the interval is what
   to quote.** Two individual sets came back 0/18 and calling that a floor was a mistake worth not
   repeating: a zero on 18 attempts has a 95% upper bound near 18% by itself, and a third set of
   fresh images then drew 4/24. Pool the sets; do not quote the lucky cell.

   **What justifies 5 anyway is that image processing stopped working.** A tooled arm was run
   specifically to decide this, with python, Pillow, numpy, a description of every deformation and
   the bundled font to template-match against:

   | arm | solved | chars | cost |
   |---|---|---|---|
   | vision Opus | 1/12 | 65% | 49k tokens, 22 calls |
   | vision Sonnet | 3/12 | 67% | 52k tokens, 16 calls |
   | tooled Opus | 3/12 | 73% | 319k tokens, 138 calls, 39 min |
   | tooled Sonnet | 0/12 | 60% | 255k tokens, 238 calls, 32 min |

   Paired per image and per model: tools won 3, looking won 4, neither solved 17 — **p = 1.0, no
   effect**, at 5-6x the token cost. On the old renderer tooling was decisive at this level (3/3
   against 2/3 vision-only) and it is what forced the default up to 8 in the first place.

   `gradient` and the post-composite blur are the reason it stopped working, and this is the clearest
   evidence either of them has produced. Hue splitting was the tooled attack's whole segmentation
   strategy — every glyph had its own random hue, so isolating a hue band isolated a letter. A letter
   no longer has one hue, and its boundary with the next letter is a hue gradient rather than a step.
   Tooled Sonnet's own summary lists "hue-based letter isolation" among the techniques it applied
   before scoring zero.

   Every arm recovers 60-73% of characters, so these are near-misses held back by case-sensitive
   validation, not failures to see the letters — that margin is thinner than the solve rate suggests.

   **The ladder above 5, measured the same way — 4 arms, 18 fresh images, 72 attempts:**

   | level | solved | characters | note |
   |---|---|---|---|
   | 5 | 7/84 = 8.3% | 66% | pooled over three sets |
   | 6 | 6/24 = 25% | 63% | |
   | 7 | 2/24 = 8.3% | 38% | |
   | 8 | 1/24 = 4.2% | 37% | |

   **Do not read the solve column as a ladder — it is not monotonic and it cannot be, at 24 attempts
   per level.** Level 6 came back higher than level 5. The character rate is the statistic to trust
   here, because it rests on 120 characters per level rather than 24 images, and it *is* monotonic:
   66, 63, 38, 37. The rendering gets steadily harder; the solve counts are too sparse to show it.

   The practical consequence is that **the difficulty dial cannot be tuned on this evidence between 5
   and 8.** Their confidence intervals overlap almost completely ([4.1%, 16.2%] against [0.7%,
   20.2%]), so picking 8 over 5 buys an unmeasurable amount of safety for 20% more render time and
   20% more stored bytes. If a deployment needs a demonstrably lower rate, the lever is not here.

   **Solution length is the lever, and it is not close.** Pooled over difficulty 5-8 and every arm:

   | length | solved | characters |
   |---|---|---|
   | 4 | 8/40 = 20% | 58% |
   | 5 | 8/40 = 20% | 64% |
   | **6** | **0/40 = 0%**, 95% CI [0%, 8.8%] | 44% |

   **Zero solves in 40 attempts at length 6**, against 20% at the current `DEFAULT_LENGTH` of 5. The
   mechanism is arithmetic rather than mysterious: solving requires every character, so the solve
   rate is roughly the per-character rate raised to the length, and these arms sit at 44-64% per
   character. It costs almost nothing — length 3 to 12 moves the median render from 6.6ms to 7.9ms,
   so 5 to 6 is about 2% — and it is far gentler on a human than cranking difficulty, since a longer
   string of legible letters beats a shorter string of mangled ones.

   **`DEFAULT_LENGTH` is therefore 6, raised from 5 on this evidence.** It is the first thing to reach
   for, ahead of difficulty and well ahead of another deformation. The cost is about 6% more stored
   bytes and one more character for the user to type; the difficulty dial would have charged 20% more
   render time for an unmeasurable gain and a real loss of legibility.

   Note what that does to the difficulty-5 figures above: they were measured across lengths 4-6, so
   they describe the *old* default's exposure. The shipped configuration now excludes the two easier
   thirds of that mix, and the honest way to read the pooled 8.3% is as an upper bound on what
   length 6 alone would give.

   Implementation note worth not undoing: rotation goes through
   `GlyphMask::displace_and_rotate`, which composes it with the existing shear-and-wave row
   displacement into a *single* resample. It cannot fold into `displace_rows` — that function
   samples on exact integer rows, which a rotation violates — but it shares the pass, because two
   bilinear resamples visibly soften a glyph. Zero rotation delegates to `displace_rows` and returns
   a byte-identical buffer, which is what keeps the difficulty-1 contract and the pinned output
   tests meaningful.
9. **Case sensitivity buys a difficulty band, and only against a solver that cannot preprocess.**
   Validation compares case-sensitively, which converts "nearly read it" into a failed solve: the
   frontier arms recovered 65-75% of individual characters while solving 1 of 48 at difficulty 8 and
   above, and several near-misses were case alone. On one difficulty-5 image every *vision-only* arm
   returned the right four letters and only lower-cased the leading `V`.

   On the renderer as it stood then, that advantage did not survive an attacker with image tools:
   tool-equipped Opus made **zero** case errors across 12 challenges where every other arm made one
   or two, and the case fix specifically took it from the 2/3 the vision arms managed at difficulty 5
   to 3/3. Cropping a glyph and viewing it enlarged beside its neighbours resolves the height cue that
   decides case.

   **That no longer reproduces.** Re-measured at difficulty 5 after rotation, the concave ramp and the
   post-composite blur, case-only misses ran 1 for tooled Opus, 1 for tooled Sonnet, 1 for vision
   Sonnet and 0 for vision Opus — the tooled arms are no longer the ones getting case right. Rotation
   is the plausible reason: the height cue that decides case is read against a shared baseline, and
   rotation is the deformation that removes one.

   Either way, keep case-sensitive matching and do not file it as the reason any difficulty holds. It
   is free and it converts near-misses into failures — the arms recover 60-73% of characters at
   difficulty 5 while solving 8% — but what holds a difficulty level is the rendering, overlap plus
   per-letter deformation, and that is where a regression would actually cost something. The flip side
   is worth stating too: a 60-73% character rate means the margin is thinner than the solve rate
   makes it look, and it is case sensitivity holding much of that gap.
10. **JPEG quality is a size choice, not a security control.** `CAPTCHA_COMPRESSION` defaults to 40.
   An earlier measurement — lossless PNG against JPEG q40 on identical pixels — did show the
   compression artifacts costing a frontier vision model a full solve, but that was on the
   renderer *before* hue randomisation and clustering, and it no longer reproduces. Re-measured on
   the current renderer at difficulty 10, quality 20/40/70/95 over identical pixels gave 3-4 of 15
   characters and zero solves at every level, across a 10x range in encoded size. The likely reason
   is that artifacts mattered while glyphs were cleanly separable by a fixed palette; now that
   colour is continuous and letters overlap, the rendering dominates and the encoder is not the
   marginal factor. Keep 40 for bandwidth and storage. Do not raise it expecting harm, or lower it
   expecting benefit, without measuring at a difficulty where solve rates are non-zero — the
   difficulty-10 test floors every model regardless of quality, so it cannot detect an effect.
11. **Airgapped Solutions**: No API returns the answer to a stored session — not the HTTP API, not the
   NAPI bindings. Use the stateless `generate()` binding if you need the plaintext without storage.

### Best Practices

- ✅ Use HTTPS in production
- ✅ Set strong, random `API_KEY_SALT`
- ✅ Rate limiting (implemented via reverse proxy support)
- ✅ Monitor for unusual patterns (multiple failed validations)
- ✅ Keep dependencies updated

## Dependencies

### Core Dependencies

```toml
axum = "0.8"                    # Web framework
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time", "sync", "signal"] }
sqlx = { version = "0.9", features = ["runtime-tokio", "sqlite", "migrate"] }

# CAPTCHA rendering. Deliberately minimal features: only JPEG is encoded.
# `image`'s defaults would add every codec (AVIF, EXR, TIFF, PNG, WebP, ...)
# and ~76 transitive crates. The drawing and noise routines are vendored in
# `src/services/captcha/drawing.rs`, so there is no `imageproc` dependency.
image = { version = "0.25", default-features = false, features = ["jpeg"] }
ab_glyph = "0.2"

serde = { version = "1", features = ["derive"] }
uuid = { version = "1", features = ["v4", "serde"] }
sha2 = "0.11"                   # Hashing
chacha20poly1305 = { version = "0.11", default-features = false, features = ["alloc", "getrandom"] }
pico-args = { version = "0.5", features = ["eq-separator"] }
toml = { version = "1", default-features = false, features = ["std", "parse", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"                 # Logging
rand = "0.10"                   # Random generation
```

### Cargo Features

| Feature | Default | Effect |
|---------|---------|--------|
| `otel`  | off     | Compiles in the OpenTelemetry OTLP trace exporter |

`otel` is off by default because the OTLP exporter pulls a full HTTP client
(reqwest and friends) into the binary — roughly 700 KB — for a path that is
inert unless `OTEL_ENABLED` is set at runtime.

**Release builds enable it explicitly.** `release.yml` and the `justfile`
both pass `--features otel` to `cross build`, so the published container
images keep the exporter. Only plain `cargo build` omits it.

Note the split: the `opentelemetry` **API** crate is an unconditional
dependency because `src/metrics.rs` builds every counter and histogram on it.
Only the SDK, the OTLP exporter and `tracing-opentelemetry` are gated, so the
instruments compile and run in every build.

**They only reach a collector in an `otel` build with `OTEL_ENABLED=true`**,
and that is worth stating plainly because the failure is silent. Instruments
are created from `global::meter()`, which binds to whatever `MeterProvider` is
installed when it is called; with none installed the global default is a no-op
that accepts every measurement and discards it. For a long time nothing
installed one, so every counter in the service was dead — compiling, running,
recording nothing. `init_telemetry` now installs a `MeterProvider` alongside
the tracer provider, and `test_a_recorded_instrument_reaches_the_exporter`
holds it: it drives a provider built the same way and asserts a recorded value
comes out of the exporter.

**Order matters and is load-bearing.** `main` calls `init_tracing` (which
lands in `init_telemetry`) at startup, well before `init_metrics`. Reversing
those two would hand every instrument the no-op meter and silently restore the
original bug — no test outside telemetry would notice.

Metrics are **pushed** over OTLP, exactly like traces; nothing scrapes this
service and there is no metrics endpoint to poll. If you want Prometheus, have
the collector re-expose them.

A binary built without `otel` warns on stderr at startup if `OTEL_ENABLED` is
set, rather than dropping traces silently.

**Both configurations must be linted**, since `cfg(not(feature = "otel"))`
paths are invisible to `--all-features`:

```bash
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --all-features -- -D warnings
```

### Image Size

**Both variants are published, both multi-arch.** `Dockerfile.multiarch`
(distroless) takes the bare tags and is the default; `Dockerfile.scratch` takes
`scratch-` prefixed ones. `Dockerfile.static` is the local-development
equivalent of the former and is not published.

Measured at `b9051ee` with the binaries built exactly as `release.yml` builds
them — `cross build --release --features otel`, both musl targets — and pushed
to a registry, so the compressed column is what a `docker pull` really transfers:

| variant | platform | pull | unpacked |
|---------|----------|------|----------|
| distroless (default) | linux/amd64 | 2,766,300 B (2.77 MB) | 7,333,888 B (7.33 MB) |
| distroless (default) | linux/arm64 | 2,716,969 B (2.72 MB) | — |
| scratch | linux/amd64 | 2,167,208 B (2.17 MB) | 4,428,800 B (4.43 MB) |
| scratch | linux/arm64 | 2,117,873 B (2.12 MB) | — |

The binary is 4,228,016 B on amd64 and 3,543,232 B on arm64; compressed it is
2,059,727 B, so it is **74% of the distroless pull and 95% of the scratch pull**.
The base is not where a remaining win is — any further size work has to happen
in the Rust build.

Scratch is 22% off the pull and 40% off unpacked, essentially all of it tzdata:
2.42 MB unpacked for a service that stores unix timestamps.

**Three numbers exist for "size" and they never agree; say which you mean.**
*Pull* is the sum of the compressed layer blobs, per platform, read from the
registry manifest — that is the table above. *Unpacked* is the flattened
filesystem, `docker export $(docker create <image>) | wc -c`. And `docker
images` reports a third, larger figure that is the overlayfs on-disk footprint
with block rounding, not layer content. Quoting the third is how the previously
advertised "7.42 MB" drifted out of date.

Do not hand-maintain these numbers. `release.yml` measures both, per variant per
platform, on every tag and writes them to the run summary; check a release
rather than trusting this table.

Both variants are verified end to end before publication, not merely built —
health, a full CAPTCHA round-trip (render, encrypt, SQLite write, decrypt,
serve), `/data` volume placement and SIGHUP reload. Scratch needs that more than
distroless does: everything distroless provides has to be hand-staged there —
`/data` and `/tmp` created in a builder stage since there is no shell, a numeric
`USER` because there is no `/etc/passwd` to resolve a name against, and the CA
bundle the OTLP exporter needs against an https collector. The CA bundle comes
from the alpine base rather than an `apk add`, so the build needs no network.

That hand-staging is why distroless stays the default: there is more to get
wrong on scratch and no shell in which to find out. The binary itself needs none
of it — it is static-pie with no libc dependency and runs on an empty filesystem.

Both `datadir`/`staging` stages are pinned with `FROM --platform=$BUILDPLATFORM`.
All they produce is a directory with an owner, two lines of passwd/group text
and a PEM bundle, none of which is architecture-specific, so building them for
the *target* would mean emulating aarch64 under qemu to run `mkdir` — slow, and
a dependency on binfmt being registered on the runner for no gain.

### SQLite Build Flags

`.cargo/config.toml` sets `LIBSQLITE3_FLAGS` to strip the bundled SQLite
amalgamation down to what the service uses — no FTS, R-tree, STAT4, JSON1,
soundex, deprecated shims or extension loading. That is ~332 KB of binary
for six query shapes that touch none of it. `SQLITE_DQS=0` additionally
rejects double-quoted string literals, so a mistyped identifier errors
instead of silently becoming a string.

**`.cargo/config.toml` is tracked on purpose, against a `.gitignore` rule that
would otherwise swallow it.** napi-rs generates its own `.cargo/config.toml`
when cross-compiling, so `.cargo/` is ignored; for a while that silently caught
this hand-written file too, and because `git add -A` reports nothing for an
ignored path, the trim above existed only on one developer's disk. Every clone,
CI run and `cross` release build compiled the full amalgamation while this
section claimed otherwise. Re-including one file from an ignored directory takes
four lines, because git does not descend into an excluded directory and a lone
negation cannot bring it back:

```gitignore
.cargo/                 # any .cargo dir, at any depth
!/.cargo/               # except the root one, so git descends into it
/.cargo/*               # but ignore what is inside it
!/.cargo/config.toml    # except this file
```

Do not "simplify" those four lines. Verify with `git check-ignore -q <path>`
(exit 0 means ignored) that the root config stays tracked while nested
`bindings/*/.cargo/` stays ignored. Confirm the flags actually reach the build
with `nm target/debug/captchapi | grep -c sqlite3_load_extension` — 0 with the
config present, 1 without it.

**Every workspace member must depend on sqlx with `sqlite-bundled`, never
`sqlite`.** The latter enables sqlx's `sqlite-load-extension` feature, whose
bindings reference `sqlite3_load_extension` — a symbol that does not exist in
a library built with `SQLITE_OMIT_LOAD_EXTENSION`. Cargo unifies features
across the workspace, so a single member requesting `sqlite` breaks the
entire build with `undefined symbol: sqlite3_load_extension`.

### Vendored CAPTCHA Renderer

Two files are vendored rather than depended on. Both exist because a crate in
the chain forces feature or dependency choices this project cannot override.

**`src/services/captcha/generator.rs`** — from `captcha-rs` v0.5.0 (MIT):

1. Upstream depends on `imageproc` with default features, whose `default`
   list includes `image/default`. Because Cargo features are additive, that
   re-enables every image codec no matter what this crate declares. Vendoring
   is the only way to hold the feature set down.
2. Upstream embeds Monotype Arial, whose license forbids redistribution. The
   renderer uses Roboto Bold (SIL OFL 1.1) instead.

**`src/services/captcha/drawing.rs`** — from `imageproc` v0.26.2 (MIT): the
text, Bézier, line, circle and noise routines, specialised to `RgbImage`.
`imageproc` declares `nalgebra` non-optionally, with no feature to switch it
off, so depending on it meant carrying `simba`, `paste` (RUSTSEC-2024-0436,
unmaintained), `matrixmultiply`, `num-complex`, `approx`, `safe_arch`,
`typenum`, `wide`, `rawpointer` and a second major version of `rand` (via
`rand_distr`) — 29 crates, none reachable from the six functions used.

Specialising to `RgbImage` is behaviour-preserving rather than approximate:
upstream's blanket `impl<I: GenericImage> Canvas for I` defines `draw_pixel`
as `put_pixel`, and the renderer never used the `Blend` wrapper that makes the
trait interesting. Only the noise generators diverge, and only in their RNG —
they sample Box-Muller from `rand` 0.10 instead of `rand_distr` over `rand`
0.9. Seeds are drawn freshly per CAPTCHA, so no output was ever reproducible
across calls and nothing observable changed.

**The geometry is pinned by digests** in `drawing.rs`'s tests, captured while
the port still ran side by side with `imageproc` under a differential test
asserting byte-identical buffers. Those digests are the only remaining record
of upstream's behaviour — the comparison cannot be re-run once the dependency
is gone, so treat a digest change as a regression until proven otherwise.

Attribution for all of it lives in `THIRD_PARTY_LICENSES` and `NOTICE`. Keep
them in sync when touching either renderer or the bundled font.

### Embedded Font

`assets/fonts/Roboto-Bold-subset.ttf` is Roboto Bold subset to the 54
characters in `BASIC_CHAR` — 8,196 bytes instead of 33,864.

**The subset locks the character set.** Adding a character to `BASIC_CHAR`
without regenerating the font makes it render as `.notdef`. The
`test_every_basic_char_has_a_glyph` unit test fails when the two drift, so
follow it up with:

```bash
pip install fonttools
python3 scripts/subset-font.py path/to/Roboto-Bold.ttf
```

Roboto declares no Reserved Font Name, so the subset keeps the family name
and needs no renaming. Swapping to a font that *does* reserve its name (most
OFL fonts, including Liberation) would reintroduce that obligation.

## Background Tasks

### Cleanup Task

Runs every 60 seconds (configurable via `CLEANUP_INTERVAL_SECONDS`):
- Deletes sessions where `expires_at < current_time`
- Logs number of sessions cleaned up
- Runs asynchronously without blocking the server

## Error Handling

All errors return JSON responses with this format:

```json
{
  "error": "error_code",
  "message": "Human-readable description"
}
```

### Common Error Codes

- `session_not_found` - Session doesn't exist or expired (404)
- `unauthorized` - Invalid or missing API key (401)
- `invalid_parameters` - Bad request parameters (400)
- `database_error` - Internal database error (500)
- `internal_error` - Other internal errors (500)

## Logging

The application uses structured logging via `tracing`:

```bash
# Set log level via environment variable
RUST_LOG=captchapi=debug,tower_http=debug cargo run

# Log levels: trace, debug, info, warn, error
```

Logs include:
- Server startup and configuration
- Session creation, validation, deletion
- API key usage (last_used_at updates)
- Cleanup task operations
- Errors and warnings

## Performance Characteristics

- **Database**: SQLite in-process (no network overhead)
- **Async**: Full async/await throughout
- **Connection Pooling**: Max 5 concurrent database connections
- **Memory**: Low footprint (no image caching, base64 on-demand)
- **Cleanup**: Periodic background task (non-blocking)

## Future Enhancements

Ideas not yet scheduled:

- Multiple CAPTCHA types (math, audio)
- Session statistics/analytics
- Horizontal scaling support
- OpenAPI/Swagger documentation
- Admin dashboard

## Troubleshooting

### Database Issues

```bash
# Delete database to reset
rm -rf data/

# Restart server (will recreate database)
cargo run
```

### API Key Issues

API keys are managed via the admin API endpoints (require master key):

```bash
# Create a new API key
curl -X POST http://localhost:3000/api/v1/api-keys \
  -H "Authorization: Bearer YOUR_MASTER_KEY" \
  -H "Content-Type: application/json" \
  -d '{"description": "My Key"}'

# List all keys
curl http://localhost:3000/api/v1/api-keys \
  -H "Authorization: Bearer YOUR_MASTER_KEY"

# Deactivate a key
curl -X PUT http://localhost:3000/api/v1/api-keys/{key_hash} \
  -H "Authorization: Bearer YOUR_MASTER_KEY" \
  -H "Content-Type: application/json" \
  -d '{"is_active": false}'

# Delete a key
curl -X DELETE http://localhost:3000/api/v1/api-keys/{key_hash} \
  -H "Authorization: Bearer YOUR_MASTER_KEY"
```

See `docs/API.md` for full API key management documentation.

### Port Already in Use

```bash
# Change port in .env
SERVER_PORT=8080

# Or use environment variable
SERVER_PORT=8080 cargo run
```

## Contributing

When making changes, follow this workflow:

### 1. Write Tests First
- ✅ Write Rust unit tests for new functionality
- ✅ Write Rust integration tests for HTTP endpoints
- ✅ Write Bruno API tests for new endpoints (in `.bruno/Tests/`)
- ✅ Add Bruno core endpoints for normal usage (in `.bruno/`)

### 2. Implement Changes
- ✅ Write the actual code
- ✅ Update relevant documentation (README.md, docs/, this file)

### 3. Verify Quality (Run IN ORDER)
- ✅ Step 1: Run `cargo fmt` to format code
- ✅ Step 2: Run `cargo clippy` to check for issues
- ✅ Step 3: Run `cargo check` to verify compilation
- ✅ Step 4: Run `cargo nextest run` to verify Rust tests pass
- ✅ Step 5: Run `./.bruno/Tests/Scripts/test-bruno-full.sh` to verify API tests (requires running server)

### 4. Finalize
- ✅ Commit with descriptive messages

**CRITICAL REQUIREMENTS:**

1. **All 5 verification steps must pass** before committing
2. **Both test suites are mandatory** - Changes may pass Rust tests but break the HTTP API
3. **Write tests BEFORE implementing features** - Test-driven development
4. **Update BOTH test suites** when making changes:
   - Rust tests (unit + integration)
   - API tests (Bruno collection)
5. **Server must be running** for API tests (Step 5)

### Test Coverage for New Features

Every new endpoint MUST have:
- ✅ Rust integration tests (success + failure cases)
- ✅ Bruno test scenarios (in `.bruno/Tests/`)
- ✅ Bruno core endpoint (in `.bruno/` root)
- ✅ Documentation in relevant files

## License

Apache-2.0

## Support

For issues, questions, or contributions, please refer to the project repository.

---

**Last Updated**: 2026-07-28
**Version**: 1.0.1
**Rust Edition**: 2021
