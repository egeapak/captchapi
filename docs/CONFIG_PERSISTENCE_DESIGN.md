# Storing configuration in SQLite, and applying boot fields by restart

Implementation plan for making every parameter changeable from the admin console — not just the
five `Reload::Live` ones — by storing values in the database and restarting the process to pick
up the rest.

Revised after the provenance work landed: `Source`, `Sources` and the pinning rule now exist,
and this feature builds on them rather than inventing its own notion of where a value came from.

## What changes for the operator

Today `PATCH /api/v1/admin/config` accepts five live fields, refuses anything pinned by the
command line or environment, and loses every accepted change on the next reload. After this:

- Any storable parameter can be **stored**, surviving restarts.
- Boot fields become settable. They do not take effect immediately; the server reports which
  ones are waiting and the console shows a pending-restart banner.
- A restart can be triggered from the console, or left to the operator. Those are separate
  decisions, and the second is off by default.

## Four constraints that bound the feature

**1. Configuration cannot fully come from the database, because opening the database is
configured.** `database_url` and `database_max_connections` are read to open the pool, so they
can never be read *from* it. `main.rs` makes this concrete: config resolves at line 27, the pool
opens at line 75.

**2. Anything used before the pool opens would apply one boot late.** Between those two lines
sit `init_tracing` (`log_level`, the three `otel_*`) and the PID file. Storing those would mean
a value that silently takes effect on the *next* restart — exactly the half-truth this codebase
refuses elsewhere. They stay unstorable unless subscriber initialisation moves after the pool,
which loses early-boot logs. See decision D2.

**3. Secrets must not be stored.** They are deliberately file-only on the CLI and absent from
the TOML schema so a checked-in config file is safe by construction. Writing them into a SQLite
file that the backup story treats as data would undo that.

**4. A field pinned by the command line or environment is not storable either.** This is new,
and follows directly from the rule already shipped: `env` outranks `stored`, so a stored value
for a field set in the environment would never become the effective one. Storing it would be
recording an override that provably cannot take effect — the same thing `ConfigHandle::patch`
already refuses to do. `PUT /config/stored` reuses `409 config_pinned`.

**This last one has a sharp edge worth stating before any code is written.** A container
deployment that passes everything through `-e` / `env:` pins everything, and gets nothing
storable. The remedy is the one the console already gives for PATCH — stop setting the field in
the environment and manage it here instead — but it means this feature is only useful to
deployments willing to hand a subset of settings over to the database. An `.env` file is fine;
it is a file, and files stay changeable.

## The storable set

`Param` gains a column:

```rust
pub enum Persist {
    /// Storable in the database.
    Allowed,
    /// Never stored: a secret, or needed before the database is open.
    Never,
}
```

11 of the 22 parameters are `Never`:

| excluded | why |
|----------|-----|
| `api_key_salt`, `master_api_key`, `solution_hash_secret`, `image_encryption_secret` | constraint 3 |
| `database_url`, `database_max_connections` | constraint 1 |
| `pid_file`, `log_level`, `otel_enabled`, `otel_endpoint`, `otel_service_name` | constraint 2 |

Leaving 11 `Allowed`: `server_host`, `server_port`, `default_session_ttl_seconds`,
`max_session_ttl_seconds`, `max_validation_attempts`, `captcha_compression`,
`cleanup_interval_seconds`, `rate_limit_requests_per_second`, `rate_limit_burst_size`,
`rate_limit_reverse_proxy`, `admin_config_write` — every live field, plus the six boot fields an
operator actually wants to tune.

Tests enforce that every `secret` param is `Never`, that the two database params are `Never`,
and that the excluded list is exactly the table above, so adding a parameter forces a decision
rather than defaulting into storability.

## Precedence

`stored` slots in below the process-level layers and above the files:

```
admin API > command line > environment > SQLite (stored) > env file > config file > default
```

The command line and environment stay on top because they are the recovery path: a stored value
that makes the service misbehave must be overridable with `captchapi --port 3000` without anyone
opening SQLite. Files sit below because they are the deployment baseline that durable operator
intent is meant to override.

Costs one `Source::Stored` variant (label `"stored"`), one field on `Layers` and `LayeredEnv`,
one arm in `LayeredEnv::get` and `source_of`. `Source::is_pinned` is **unchanged** — `Stored` is
not pinned, because it is precisely the layer this feature makes changeable.

## Schema

`migrations/20260727000000_config_store.sql`:

```sql
CREATE TABLE config_settings (
    field       TEXT PRIMARY KEY,   -- Config field name, e.g. 'server_port'
    value       TEXT NOT NULL,      -- raw string, parsed by the same code every layer uses
    updated_at  INTEGER NOT NULL,
    updated_by  TEXT NOT NULL       -- 'admin-api' | 'cli'
);

-- One row per attempt to boot with changed stored config, so a configuration that prevents
-- startup rolls back automatically instead of wedging the service.
CREATE TABLE config_generations (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at INTEGER NOT NULL,
    snapshot   TEXT NOT NULL,       -- JSON map of config_settings as of this generation
    status     TEXT NOT NULL,       -- 'pending' | 'confirmed' | 'rolled_back'
    attempts   INTEGER NOT NULL DEFAULT 0
);
```

Values are raw strings in the same canonical form every other layer produces, so parsing and
validation stay in `Config::from_env_provider` and nowhere else. Keyed by `field`, matching what
the admin API already speaks.

## Boot sequence

```
1. Parse argv, resolve from cli/env/env-file/file                    (unchanged)
2. init_tracing                                                      (unchanged)
3. Open the pool, run migrations                                     (unchanged)
4. Generation bookkeeping:
     newest generation is 'pending' and attempts >= 1
       -> restore config_settings from the newest 'confirmed' snapshot
          (or empty, if there is none), mark it 'rolled_back', log at warn
     otherwise -> attempts += 1
5. Read config_settings into a Layer
6. Re-resolve with the stored layer inserted -> (Config, Sources)
     on failure: log at error, drop the stored layer, keep step 1's config
7. Build the handle from the step-6 result and carry on               (unchanged from here)
8. After 30s of serving, mark the newest 'pending' generation 'confirmed'
```

Step 6 is why the two-phase shape is unavoidable, and step 4's `attempts` counter is what
distinguishes "first try at a new configuration" from "we already tried this and did not
survive". Nothing between steps 1 and 6 may read a storable field — which is constraint 2,
enforced by the `Persist::Never` list rather than by hope.

## Changes to `ConfigHandle`

`reload()` currently re-reads files. It must now also re-read the store, but it is a sync
function called under `spawn_blocking` and sqlx is async. Rather than give the handle a database
connection, the caller fetches and passes:

```rust
pub fn reload(&self, stored: Layer) -> Result<Outcome, String>
```

`ReloadState` retains the layer so `patch()` re-resolves against the same stored values without
another round trip. Call sites: the SIGHUP task, `POST /config/reload`, and the tests — all
already async or trivially able to supply an empty layer.

This keeps the handle database-agnostic, which is what makes it testable without a pool, and it
puts freshness in the type system: you cannot reload without having decided what the store says.

## The restart mechanism

Re-exec, as evaluated in the previous revision of this document and unchanged: at the end of
`main`, after graceful shutdown, replace the process image with a fresh copy of itself. Same
PID, so `docker stop`, Kubernetes, systemd and the PID file keep working untouched, and no
supervisor inherits PID 1's reaping and signal-forwarding obligations.

Concretely:

- **Capture `args_os()` at startup**, before parsing consumes it, and keep it for the exec. The
  environment carries across `execv` automatically, preserving the env layer.
- **Trigger**: a `RestartHandle { flag: AtomicBool, token: CancellationToken }` in `AdminState`.
  `POST /admin/restart` sets the flag and cancels the token; `shutdown_signal` gains a third
  select arm on it, so the existing graceful-shutdown path runs unchanged.
- **Order at the end of `main`**: stop background tasks, **close the pool** (so WAL is
  checkpointed rather than left for recovery — SQLite's VFS already opens with `O_CLOEXEC`, so
  this is about flushing, not leaking), flush telemetry, **skip PID-file removal** because the
  PID does not change, then exec from the main thread with the runtime finished.
- **`current_exe()` guard**: on Linux this resolves `/proc/self/exe`, which yields a path
  suffixed `(deleted)` if the binary was replaced on disk. If the path does not exist, log at
  error and exit normally instead — a service that exits is recoverable by an orchestrator; one
  that execs a nonexistent path is not.

In-flight requests are dropped at the exec boundary. True zero-downtime needs overlapping
processes with socket handoff, which is a separate and much larger feature.

## Not restarting into a broken configuration

Four layers, cheapest first:

1. **Validate** the candidate through `Config::from_env_provider` before writing anything.
   Catches every type and range error. `400 invalid_config`.
2. **Pre-flight the resource checks validation cannot do.** If `server_host`/`server_port`
   changed, bind the new address and release it; `409 address_unavailable` if taken. This is the
   realistic failure — "port already in use" is exactly what someone editing `server_port` in a
   web form hits, and it is invisible to validation. Skipped when the address is unchanged,
   since this process already holds it.
3. **Generations with automatic rollback**, per the boot sequence. Covers whatever the first two
   missed.
4. **Escape hatches needing no database access**: `--ignore-stored-config` (also
   `IGNORE_STORED_CONFIG=true`), plus `captchapi config unset <field>` and
   `captchapi config clear` for offline repair. These matter more than usual here: the
   distroless and scratch images have no shell and no `sqlite3`, so without them a bad stored
   value could only be fixed by rebuilding the image.

Storing `admin_config_write = false` locks the console out of its own settings. The CLI hatch
covers it; the console should confirm before allowing it.

## API surface

The durable store becomes its own resource rather than overloading `PATCH`, so today's
ephemeral-override semantics keep working unchanged:

| method | path | effect |
|--------|------|--------|
| `GET` | `/api/v1/admin/config` | as today, plus `storable` and `pending_restart` |
| `PATCH` | `/api/v1/admin/config` | **unchanged** — ephemeral, live fields, cleared by reload |
| `GET` | `/api/v1/admin/config/stored` | the stored layer; secrets absent by construction |
| `PUT` | `/api/v1/admin/config/stored` | store fields; live ones apply at once, boot ones land in `pending_restart` |
| `DELETE` | `/api/v1/admin/config/stored/{field}` | remove one stored field |
| `POST` | `/api/v1/admin/restart` | graceful shutdown then re-exec; **off by default** |

`ConfigEntry` gains `storable: bool` alongside the existing `editable: bool`. The two are
different questions and the console needs both:

- `editable` — `PATCH` will take it: live, and not pinned.
- `storable` — `PUT /stored` will take it: `Persist::Allowed`, and not pinned.

A live field can be both. A boot field can be storable but never editable. Neither is true for
anything pinned or secret.

`PUT` re-resolves and publishes immediately, so a stored live field takes effect at once — the
same way `PATCH` does — while a stored boot field only changes what a restart would produce.
`pending_restart` is computed by comparing each stored boot field against the running config.

`POST /config/reload` re-reads the store as well as the files: the store is now a source of
truth, and reload means "re-read the sources". It still clears the ephemeral overlay.

New `AppError` variants — `ConfigNotPersistable` (400), `RestartNotEnabled` (403),
`AddressUnavailable` (409) — **will break `bindings/nodejs`**, which matches `AppError`
exhaustively with no wildcard arm. Build with `--workspace`.

New parameter: `ADMIN_RESTART_ENABLED` (bool, `Reload::Boot`, default `false`). A remote restart
endpoint is an availability lever and a DoS amplifier if the master key ever leaks, so it is
opt-in, master-key only, audit-logged and counted in metrics. `ADMIN_CONFIG_WRITE=false`
disables the stored writes too, matching how it already disables `PATCH`.

## Console changes

- A **stored value** column, distinct from the effective one, and a `stored` tag for rows whose
  effective value comes from the store.
- **Boot rows become editable** when storable, tagged `restart to apply` rather than
  `restart to change`.
- A **pending-restart banner** listing the waiting fields, with a Restart button when the
  endpoint is enabled.
- Apply gains a **"store these changes"** checkbox. A per-row toggle is too fussy for 11 fields;
  one checkbox next to Apply chooses between ephemeral `PATCH` and durable `PUT`.
- A confirm step before storing `admin_config_write = false`.

Budget: roughly +3 KB of assets, keeping the pair under 20 KB.

## Work breakdown

Ordered so each step compiles, passes, and is independently reviewable.

1. **`Persist` column and the storable set** — `params.rs`, plus the three invariant tests.
   No behaviour change.
2. **`Source::Stored` and the layer** — `sources.rs`, `cli.rs` (`Layers.stored`), unit tests for
   precedence and for `Stored` not being pinned.
3. **`ConfigStore` service** — `src/services/config_store.rs`: read the layer, upsert, delete,
   and the generation bookkeeping. Tests against an in-memory pool, as the other services do.
4. **Two-phase boot** — `main.rs` steps 4-6 plus the confirm task, and `ConfigHandle::reload`
   taking a `Layer`. Integration test that a stored value survives a simulated restart.
5. **Stored endpoints** — `GET`/`PUT`/`DELETE /config/stored`, `storable` and `pending_restart`
   on `GET /config`, the new error variants and the NAPI arms. Rust integration tests + Bruno.
6. **Rollback** — generations table wired into boot, with a test that writes a `pending`
   generation with `attempts = 1` and an unbootable value and asserts it rolls back.
7. **Restart** — `ADMIN_RESTART_ENABLED`, `POST /admin/restart`, the shutdown-token arm,
   pre-flight bind check, and the exec itself.
8. **CLI hatches** — `--ignore-stored-config`, `config unset`, `config clear`.
9. **Console** — stored column, pending banner, restart button, store checkbox; Playwright
   coverage for each.
10. **Docs** — `docs/API.md`, `CLAUDE.md` (precedence, schema, the new parameter),
    `.env.example`, `captchapi.toml.example`, and the size table in
    `docs/ADMIN_UI_EVALUATION.md`.

## Testing

The existing gates cover most of it. Three things need new kinds of test:

- **Precedence and storability** — unit tests on the layer stack, that a `Persist::Never` field
  is refused, and that a field pinned by cli/env is refused with `config_pinned`.
- **Rollback** — write a `pending` generation with `attempts = 1` and a value that cannot boot,
  then assert the boot path restores the last confirmed snapshot. No process machinery needed.
- **Re-exec** — not unit-testable. Needs an integration test that spawns the real binary, stores
  a boot field, calls restart, and re-queries on the new image asserting the PID is unchanged.
  This is the one genuinely new harness, and the one most likely to be flaky in CI; it should be
  `#[ignore]`d by default and run explicitly if it proves unstable.

Plus Bruno coverage for the four new endpoints and Playwright coverage for the console changes.

## Decisions taken, and the ones worth confirming

**D1 — restart drops in-flight requests.** Accepted. The alternative is overlapping processes
with socket handoff, a separate feature several times this size.

**D2 — `log_level`, `pid_file` and the `otel_*` trio stay unstorable.** Recommended: the
alternative is moving subscriber initialisation after the pool opens and losing early-boot logs,
which is a bad trade for three rarely-changed settings. Worth confirming, since it means the
console cannot change the log level — arguably the setting an operator most wants to change
live.

**D3 — `PUT /config/stored` does not restart.** Storing and restarting stay separate calls, so
storing several fields costs one restart rather than several.

**D4 — pinned fields are not storable.** This follows from precedence and from the rule already
shipped, but it is the decision most likely to disappoint: a fully env-driven container
deployment gets nothing storable. Called out above under constraint 4.

**Scope.** This roughly doubles the pull request, which is already +2015/−33 across 27 files. It
is a coherent unit — the console is the reason to want stored boot fields — but it is a lot to
review at once, and steps 1-6 are useful without steps 7-9. Splitting at that seam is worth
considering if review latency matters more than shipping it whole.
