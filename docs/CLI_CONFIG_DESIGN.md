# CLI & Dynamic Configuration — Design

Status: **implemented.** This is the design record — it exists to explain *why* the shape is
what it is. For how the shipped system actually works, see
[ARCHITECTURE.md](ARCHITECTURE.md#configuration) and the
[Configuration section of the README](../README.md#configuration). Where this document and the
code disagree the code wins; the decisions that changed during implementation are listed in
[§12](#12-where-the-implementation-diverged).

This document describes adding a command-line surface (`pico-args`), a TOML config
file layer, and runtime-reloadable configuration to CaptchAPI.

---

## 1. How configuration works today

| Concern | Where | Notes |
|---|---|---|
| Resolution | `src/config.rs` — `Config::from_env()` | 14 fields, each parsed inline with `unwrap_or_else(default) → parse() → map_err("Invalid X")` |
| Test seam | `EnvProvider` trait | `RealEnv` in prod, `MockEnv` in tests. **This is the seam the CLI plugs into.** |
| `.env` loading | `src/main.rs:26` — `dotenvy::dotenv()` | Runs before anything else |
| OTEL settings | `src/telemetry.rs` | Reads `std::env` directly — *bypasses `Config`* |
| Log level | `tracing_subscriber::EnvFilter` | Reads `RUST_LOG` directly — *also bypasses `Config`* |
| Consumers | `SessionsState.config: Arc<Config>`, `start_cleanup_task(interval_seconds)`, `build_app` | Config is read once at boot and never changes |

Three independent config readers exist. Everything is boot-time and immutable.

### Constraints this design respects

- **Binary size.** `opt-level = "z"`, full LTO, `panic = "abort"`, 7.42 MB image. Every
  dependency is scrutinised.
- **`Config` is constructed as a struct literal** in `src/app.rs:190` and
  `tests/common/mod.rs:71`. Adding fields breaks both — fixed here with a `Default` impl.
- **The NAPI bindings never touch `Config`** (only `SessionConfig`), so `bindings/nodejs`
  and `packages/captchapi` are unaffected.
- **Distroless runtime**: no shell, no `pidof`, read-only except `/data` — note the working
  directory is `/app`, so a *relative* `./data` path does **not** land in the volume.

---

## 2. Core idea: one parameter table drives everything

A single static table is the source of truth for every knob. Help text, TOML parsing,
provenance reporting, and the reloadable/boot-only partition are all derived from it.
Adding a parameter is **one table row plus one struct field**.

```rust
// src/config/params.rs
pub struct Param {
    pub env:    &'static str,          // "SERVER_PORT"          — canonical key
    pub flag:   &'static str,          // "--port"
    pub short:  Option<&'static str>,  // "-p"
    pub toml:   &'static str,          // "server.port"
    pub kind:   Kind,                  // Str | Num | Bool | SecretFile
    pub reload: Reload,                // Boot | Live
    pub secret: bool,                  // redact in `config show` / admin GET
    pub default:&'static str,
    pub help:   &'static str,
}

pub const PARAMS: &[Param] = &[ /* 20 rows */ ];
```

### Layering: the CLI is just another `EnvProvider`

Rather than a parallel CLI→Config mapping (which would duplicate all 14 parse/validate
blocks), every layer resolves the *same canonical keys*. `Config::from_env_provider`
stays byte-for-byte unchanged — including every error message and all 30 existing
config tests.

```rust
// src/config/sources.rs
// precedence: CLI flag > process env (incl. .env) > TOML file > built-in default
impl<E: EnvProvider> EnvProvider for LayeredEnv<'_, E> {
    fn get(&self, key: &str) -> Result<String, env::VarError> {
        self.cli.get(key).cloned()
            .or_else(|| self.env.get(key).ok())
            .or_else(|| self.file.get(key).cloned())
            .ok_or(env::VarError::NotPresent)
    }
}
```

TOML is parsed into `toml::Value`, then flattened to canonical keys **through `PARAMS`**.
Unknown paths are a hard error, so a typo like `[server] prot = 8080` fails loudly at
boot instead of silently doing nothing.

---

## 3. CLI surface

```
captchapi [OPTIONS]                       # run (default verb)
captchapi run [OPTIONS]
captchapi reload [--pid <N> | --pid-file <PATH>]
captchapi config show [OPTIONS]           # effective config + provenance, secrets redacted
captchapi config check [OPTIONS]          # validate and exit
captchapi --help | --version
```

| Flag | Canonical key | Reload |
|---|---|---|
| `-c, --config <PATH>` | — | boot |
| `--env-file <PATH>` / `--no-env-file` | — | boot |
| `-H, --host <ADDR>` | `SERVER_HOST` | boot |
| `-p, --port <PORT>` | `SERVER_PORT` | boot |
| `-d, --database-url <URL>` | `DATABASE_URL` | boot |
| `--database-max-connections <N>` | `DATABASE_MAX_CONNECTIONS` | boot |
| `--api-key-salt-file <PATH>` | `API_KEY_SALT` | boot |
| `--master-api-key-file <PATH>` | `MASTER_API_KEY` | boot |
| `--rate-limit-rps <N>` | `RATE_LIMIT_REQUESTS_PER_SECOND` | boot |
| `--rate-limit-burst <N>` | `RATE_LIMIT_BURST_SIZE` | boot |
| `--rate-limit-reverse-proxy` | `RATE_LIMIT_REVERSE_PROXY` | boot |
| `--otel` / `--otel-endpoint` / `--otel-service-name` | `OTEL_*` | boot |
| `--pid-file <PATH>` | `PID_FILE` | boot |
| `--default-session-ttl <SECS>` | `DEFAULT_SESSION_TTL_SECONDS` | **live** |
| `--max-session-ttl <SECS>` | `MAX_SESSION_TTL_SECONDS` | **live** |
| `--max-validation-attempts <N>` | `MAX_VALIDATION_ATTEMPTS` | **live** |
| `--captcha-compression <1-100>` | `CAPTCHA_COMPRESSION` | **live** |
| `--cleanup-interval <SECS>` | `CLEANUP_INTERVAL_SECONDS` | **live** |
| `--log-level <LEVEL>` | `RUST_LOG` | **live** |

**Secrets are file-only.** No `--api-key-salt=<value>` / `--master-api-key=<value>` —
those land in `ps aux`, shell history, and `docker inspect`. The `*-file` flags read from
disk and trim trailing whitespace, which is what Docker/Kubernetes secrets want:

```sh
captchapi --master-api-key-file /run/secrets/master_key \
          --api-key-salt-file   /run/secrets/salt
```

`config show` output — the thing that makes a layered system debuggable:

```
server_port              = 8080                    [cli]
database_url             = sqlite:./data/x.db      [env]
captcha_compression      = 75                      [file: captchapi.toml]
max_validation_attempts  = 3                       [default]
api_key_salt             = <redacted, 32 bytes>    [env]
```

**Exit codes:** `0` success · `1` runtime error · `2` usage or config error.

### TOML file

```toml
# captchapi.toml
[server]
host = "0.0.0.0"
port = 8080

[database]
url             = "sqlite:./data/captchapi.db"
max_connections = 5

[captcha]
default_ttl_seconds     = 300
max_ttl_seconds         = 3600
max_validation_attempts = 3
compression             = 75

[rate_limit]
requests_per_second = 2
burst_size          = 10
reverse_proxy       = false

[tasks]
cleanup_interval_seconds = 60

[telemetry]
enabled      = false
endpoint     = "http://localhost:4318"
service_name = "captchapi"
```

Secrets are deliberately **not** representable in the TOML file — they come from env vars
or `--*-file` flags only. This keeps a checked-in config file safe by construction.

---

## 4. Startup order in `main.rs`

Reordering is required: `--log-level` and `--otel` must be known *before* the subscriber is
built, because a `tracing` registry can only be initialized once.

1. `cli::parse()` → `--help`/`--version` print and exit `0`; bad usage → `eprintln!` + exit `2`.
2. Dispatch non-`run` verbs (`reload`, `config show`, `config check`) and exit.
3. Load env file (`--env-file`, else `.env`, unless `--no-env-file`).
4. Load TOML file if `--config` given.
5. Build `Config` from `LayeredEnv`. Errors → `eprintln!` + exit `2`.
   *This is strictly better than today, where a config error is logged through a
   subscriber that has just been configured with defaults nobody asked for.*
6. Init tracing/OTEL **from `Config`** (folding `RUST_LOG` and `OTEL_*` into `Config`, so
   all configuration finally lives in one struct).
7. Write the PID file (warn and continue on failure — never fail boot over it).
8. Existing flow: pool → migrations → metrics → `build_app` → cleanup task → serve.

---

## 5. Runtime reload

### Sharing mechanism: `tokio::sync::watch` (no new dependency)

`tokio` is already compiled with the `sync` feature. `watch::Receiver<Arc<Config>>` gives
cheap reads, and — unlike `ArcSwap` — gives the cleanup task change *notification* for free,
which it needs to pick up a new `cleanup_interval_seconds`.

```rust
#[derive(Clone)]
pub struct ConfigHandle {
    rx: watch::Receiver<Arc<Config>>,
    tx: Arc<watch::Sender<Arc<Config>>>,
    resolver: Arc<ResolverInputs>,   // retained CLI args + file path, for re-resolution
    overlay: Arc<Mutex<Overlay>>,    // admin PATCH values
}

impl ConfigHandle {
    pub fn get(&self) -> Arc<Config>;                       // per-request read
    pub fn from_static(cfg: Arc<Config>) -> Self;           // tests — no reload machinery
    pub fn reload(&self) -> Result<Arc<Config>, String>;    // re-read sources, clear overlay
    pub fn patch(&self, kv: &[(String, String)]) -> Result<Arc<Config>, String>;
}
```

`Config` itself stays a whole struct, so **existing tests and struct literals keep working**.
`SessionsState.config: Arc<Config>` becomes `ConfigHandle`, and handlers call
`state.config.get()` once at entry — a reload mid-request can never tear a handler's view.

### What reloads, and what does not

| Reloadable | Boot-only (warned about, then ignored) |
|---|---|
| `DEFAULT_SESSION_TTL_SECONDS` | `SERVER_HOST`, `SERVER_PORT` — listener is already bound |
| `MAX_SESSION_TTL_SECONDS` | `DATABASE_URL`, `DATABASE_MAX_CONNECTIONS` — pool is live |
| `MAX_VALIDATION_ATTEMPTS` | `API_KEY_SALT` — would invalidate every stored key hash |
| `CAPTCHA_COMPRESSION` | `MASTER_API_KEY` — baked into middleware at `build_app` |
| `CLEANUP_INTERVAL_SECONDS` | `RATE_LIMIT_*` — `GovernorLayer` is built once into the router |
| `RUST_LOG` | `OTEL_*` — tracer provider is registered globally at startup |

Reload re-resolves the *full* config, then copies boot-only fields from the original.
If a boot-only value differs from what the sources now say, it logs a warning naming the
field — so an operator who edits `port` and sends SIGHUP gets told why nothing happened,
rather than silently believing it worked.

**A failed reload never disturbs the running server**: the error is logged, the current
config is kept, and the admin endpoint returns `400`.

### Three triggers, one code path

1. **`SIGHUP`** — handler in `main.rs` (`SignalKind::hangup`, `cfg(unix)`), alongside the
   existing SIGTERM handling. `docker kill -s HUP <container>` works out of the box.
2. **`POST /api/v1/admin/config/reload`** — master-key protected, works on all platforms.
3. **`captchapi reload`** — resolves a PID from `--pid` > `--pid-file` > default
   `./data/captchapi.pid` and sends SIGHUP. This is the "CLI internally triggers SIGHUP"
   path; it is `cfg(unix)` and errors cleanly elsewhere.

### Log-level reload

`tracing_subscriber::reload::Layer` wraps the `EnvFilter`; the handle lives in a `OnceLock`
and `reload()` calls `handle.modify()`. No new dependency — `tracing-subscriber` is already
present.

### Cleanup task

Signature changes from `interval_seconds: u64` to a `ConfigHandle`. Its `select!` gains a
`config_rx.changed()` arm that rebuilds the `time::interval` when
`cleanup_interval_seconds` changes.

---

## 6. Admin API

All three routes sit behind the existing `MasterKeyMiddleware` on `/api/v1/admin`.

| Route | Behaviour |
|---|---|
| `GET /api/v1/admin/config` | Effective config with per-key `source` and `reloadable` flags. **Secrets are always redacted** — `{"value": "<redacted>", "length": 32}` — even for the master key holder. |
| `PATCH /api/v1/admin/config` | `{"default_session_ttl_seconds": 600}`. Runs the same validation as boot. Boot-only keys → `400 config_not_reloadable`. Invalid values → `400 invalid_config`. |
| `POST /api/v1/admin/config/reload` | Re-reads CLI + env + TOML, **clears the PATCH overlay**, returns the new effective config. |

Two new `AppError` variants: `config_not_reloadable`, `invalid_config`.

### Design decision: the PATCH overlay is ephemeral

`PATCH` values live in memory only. They are **not** written back to the TOML file (the
server may not own it — the distroless image is read-only outside `/data`), and they are
**cleared by any reload**, since "reload" means re-reading the sources of truth. This is
the least-surprising rule, but it must be documented prominently: a PATCH survives until
the next SIGHUP or restart, and no longer.

*As implemented:* `GET /admin/config` reports overridden fields in an `overrides` array rather
than as an `[admin]` provenance source — `config show` runs in a separate process and cannot
see another process's overrides, so a source label there would have been meaningless.

### Hardening note

`PATCH` is an authenticated remote mutation of security-relevant parameters
(`max_validation_attempts`, TTLs). Mitigations as implemented: master-key only; every change
audit-logged at `info` with field, old value and new value, redacted through the same helper
the API uses; and a boot-only `ADMIN_CONFIG_WRITE=false` escape hatch (flag form
`--admin-config-write=false`) that makes `PATCH` return `403` while `GET` and
`POST /config/reload` keep working.

*Not implemented:* request-ID correlation in the audit line. `request_id_middleware` only sets
a response header — it opens no tracing span — so there is nothing for a log line to inherit.
Adding it means threading the ID into a span, which is worth doing but was out of scope here.

---

## 7. Dependencies

| Crate | Why | Cost |
|---|---|---|
| `pico-args = "0.5"` | CLI parsing | zero transitive deps, ~15 KB |
| `toml`, `default-features = false, features = ["parse"]` | config file | ~200 KB. No serde derive needed — values map through `PARAMS`. |

Nothing else. `tokio::sync::watch` replaces an `arc-swap` dependency;
`tracing_subscriber::reload` is already available; binary smoke tests use
`env!("CARGO_BIN_EXE_captchapi")` rather than a new dev-dependency.

Binary size must be measured before/after with `just build amd64` and the delta recorded
in the PR — this repo advertises its image size in the README.

---

## 8. Files affected

**New**
```
src/cli.rs                  pico-args parsing, generated help, verb dispatch, SIGHUP client
src/config/mod.rs           (was src/config.rs) Config, resolution, ConfigHandle, reload
src/config/params.rs        the PARAMS table
src/config/sources.rs       LayeredEnv, TOML loader, provenance tracking
captchapi.toml.example
docs/CLI_CONFIG_DESIGN.md   this file
```

**Modified**
```
src/main.rs                 startup reorder, verb dispatch, SIGHUP handler, PID file
src/app.rs                  ConfigHandle threading
src/routes/sessions.rs      SessionsState.config → ConfigHandle
src/routes/admin.rs         three new routes
src/tasks/cleanup.rs        dynamic interval
src/error.rs                two new variants
src/telemetry.rs            take Config instead of reading env directly
src/lib.rs                  pub mod cli
tests/common/mod.rs         ConfigHandle::from_static + Config::default()
Cargo.toml                  pico-args, toml
README.md · .env.example · docs/API.md · docs/ARCHITECTURE.md · .claude/CLAUDE.md · CHANGELOG.md
.bruno/                     core requests + tests for the three admin routes
```

`impl Default for Config` is added so `app.rs` and `tests/common/mod.rs` can write
`Config { server_port: 3000, ..Default::default() }` — the next added field won't break
them again.

---

## 9. Verification

Per `CLAUDE.md`, in order: `cargo fmt` → `cargo clippy` → `cargo check` →
`cargo nextest run` → `./.bruno/Tests/Scripts/test-bruno-full.sh`.

New tests:

- **Layering** — CLI beats env beats file beats default; partial overrides; secret-file reading.
- **CLI parsing** — every flag; unknown flag → exit 2; missing value; `--help`/`--version` exit 0; verb dispatch.
- **TOML** — valid file, unknown key rejected, type mismatch, missing file.
- **Redaction** — `config show` and `GET /admin/config` never emit salt or master key.
- **Reload** — live field changes take effect; boot field change is ignored *and* warned; invalid reload keeps the old config; overlay cleared by reload.
- **Cleanup task** — picks up a changed interval without restart.
- **Admin routes** — success, `config_not_reloadable`, `invalid_config`, unauthorized (Rust + Bruno).
- **Binary smoke** — `--help`, `config check` on a good and a bad config, via `env!("CARGO_BIN_EXE_captchapi")`.

---

## 10. Suggested phasing

Four reviewable commits on `claude/cli-config-interface-design-ycbm87`:

1. **Params table + layering + CLI** — `PARAMS`, `LayeredEnv`, TOML loader, all flags,
   `config show` / `config check`, `Config::default()`. No behaviour change for existing
   deployments: env-only setups resolve identically.
2. **Reload plumbing** — `ConfigHandle`, `watch` channel, SIGHUP handler, `captchapi reload`,
   PID file, dynamic cleanup interval, log-level reload.
3. **Admin API** — `GET`/`PATCH`/`reload` routes, error variants, audit logging, Bruno tests.
4. **Docs & size** — README tables, `.env.example`, `captchapi.toml.example`, `docs/API.md`,
   `CLAUDE.md`, `CHANGELOG.md`, measured binary-size delta.

Phase 1 is independently useful and independently revertable; phases 2–3 are where the
behavioural risk lives.

---

## 11. Open questions

1. **PID file default.** Proposed `./data/captchapi.pid` — the one directory writable in the
   distroless image. Alternative: no PID file unless `--pid-file` is passed, and
   `captchapi reload` then requires `--pid`.
2. **`ADMIN_CONFIG_WRITE` default.** Proposed `true` (the feature was explicitly requested);
   the conservative alternative is `false`, making remote writes opt-in while SIGHUP reload
   stays always-on.
3. **TOML section naming.** The proposal renames keys into sections
   (`captcha.default_ttl_seconds` ← `DEFAULT_SESSION_TTL_SECONDS`). A flat
   `DEFAULT_SESSION_TTL_SECONDS = 300` file would be a more literal mapping but reads worse.

Resolved: (1) the PID file is written to `./data/captchapi.pid` by default, overridable with
`--pid-file`, and removed on graceful shutdown so a stale file cannot make `captchapi reload`
signal a recycled process; (2) admin writes are enabled by default, with an `ADMIN_CONFIG_WRITE`
opt-out; (3) TOML uses grouped sections.

**Correction to (1).** The claim below that `./data` is "the one directory writable in the
distroless image" was wrong: the image sets `WORKDIR /app`, so a relative `./data` resolves to
`/app/data`, not to the `/data` volume. Verified by simulation — with the relative default the
mounted volume received zero files. Both Dockerfiles now set `DATABASE_URL` and `PID_FILE` to
absolute `/data/...` paths; the relative defaults remain correct for local development.

---

## 12. Where the implementation diverged

Five decisions changed once the code met reality. Each is load-bearing.

1. **Log level is boot-only, not reloadable.** Making `RUST_LOG` live requires
   `tracing_subscriber::reload::Layer`, whose `register_callsite` returns
   `Interest::sometimes()`. That permanently disables per-callsite interest caching for the
   whole subscriber, so *every* request in *every* deployment pays for a feature almost nobody
   uses — a bad trade in a service built with `opt-level = "z"`. It also has an awkward type:
   `reload::Handle` is generic over the subscriber, and the OTEL-on and OTEL-off paths build
   different stacks.

2. **`dotenvy::dotenv()` is gone, replaced by an env-file *layer*.** The original plan kept the
   existing call. But `dotenv()` mutates the process environment once and never overwrites what
   is already set, which means editing `.env` and sending SIGHUP would silently do nothing while
   editing the TOML file worked. Parsing the file into its own layer — below the process
   environment, above the TOML file — preserves today's precedence exactly, makes `--env-file`
   reloadable, and removes a global `set_var` that tests would otherwise race on.

3. **No `impl Default for Config`.** `api_key_salt` and `master_api_key` are the two required
   parameters; a `Default` yielding empty strings would silently build an `AuthService` with an
   empty salt. Replaced by a named `Config::for_test()` with valid placeholders — `pub`, not
   `#[cfg(test)]`, because `tests/common/mod.rs` is a separate crate.

4. **Provenance is a pure `source_of()` query, not something recorded during `get()`.**
   `EnvProvider::get` takes `&self`, so recording would need interior mutability; a `RefCell`
   would make `LayeredEnv` `!Sync` and break the moment a reload runs inside an async task.

5. **A `carried` layer sits below the config file.** Re-resolution during a reload would
   otherwise hard-fail when a secret file has been rotated away since startup — for a field
   that is boot-only and would have been discarded anyway. Seeding the running config's
   boot-only values as the lowest-precedence layer fixes that, and has a second consequence
   worth knowing: an unspecified boot field resolves to its running value, so "drift" is
   reported only when a source *explicitly* names a different value.

Also worth recording: `main.rs` was collapsed onto the library crate. It had been re-declaring
the whole module tree, compiling everything twice into two distinct sets of types — which would
have made `mod cli;` a source of confusing type errors, and keeps `main.rs` thin enough that the
85% coverage gate stays comfortable.
