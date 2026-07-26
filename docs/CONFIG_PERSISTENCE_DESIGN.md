# Persisting configuration in SQLite, and applying boot fields by restart

Design for making every parameter changeable from the admin console — not just the five
`Reload::Live` ones — by storing values in the database and restarting the process to pick up
the rest. Nothing here is built yet.

## What changes for the operator

Today `PATCH /api/v1/admin/config` rejects boot fields outright, and any live field it accepts
is lost on the next reload or restart. After this:

- Any non-secret, non-bootstrap parameter can be **stored**, surviving restarts.
- Boot fields become editable. They do not take effect immediately; the server reports that a
  restart is pending and lists exactly which fields are waiting.
- A restart can be triggered from the console, or left to the operator — the two are separate
  decisions and the second is off by default.

## Two constraints that shape everything

**1. Configuration cannot fully come from the database, because opening the database is
configured.** `database_url` and `database_max_connections` are read to open the pool, so they
can never be read *from* the pool. `main.rs` makes this concrete: config is resolved at line 27,
the pool is created at line 75. Any stored layer necessarily loads after a first resolution
has already happened, which forces a two-phase boot (below) and puts a hard floor under what
is storable.

**2. Secrets must not be stored.** `api_key_salt`, `master_api_key`, `solution_hash_secret` and
`image_encryption_secret` are deliberately file-only on the CLI and absent from the TOML schema,
so a checked-in config file is safe by construction. Writing them into a SQLite file that also
holds session rows — and that the backup story treats as data, not credentials — would undo
that. They stay unstorable, enforced by the same `param.secret` flag and a `PARAMS` invariant
test.

So `Param` gains a column:

```rust
pub enum Persist {
    /// Storable in the database.
    Allowed,
    /// Never stored: a secret, or needed before the database is open.
    Never,
}
```

with tests asserting every `secret` param is `Never`, and that `DATABASE_URL` and
`DATABASE_MAX_CONNECTIONS` are `Never`. That is 4 + 2 = 6 of 22 parameters excluded; the other
16, including all five live ones, become storable.

## Precedence

The stored layer slots in below the process-level layers and above the files:

```
command line > environment > SQLite (stored) > env file (.env) > config file > default
```

The command line and the environment stay on top **because they are the recovery path**. If a
stored value makes the service misbehave, `captchapi --port 3000` or `SERVER_PORT=3000` must
still win without anyone having to open SQLite. Files sit below because they are the deployment
baseline that durable operator intent is meant to override.

This costs one variant on `Source` (`Stored`, label `"stored"`), one field on `Layers` and
`LayeredEnv`, and one `.or_else` in `LayeredEnv::get` and `source_of`.

**Shadowing must be reported, not swallowed.** If someone stores `server_port = 8080` while
`SERVER_PORT=3000` is in the environment, the stored value is real but not effective. The
codebase already refuses to pretend elsewhere — `ConfigHandle::patch` rejects boot fields rather
than recording an override that cannot take effect, and a reload reports boot drift rather than
applying it. The same standard applies here: the config API grows a `source` field per
parameter, and the console shows stored-but-shadowed rows explicitly. Silently storing a value
that a higher layer overrides is the one failure mode that would make this feature actively
misleading.

## Schema

A new migration, `migrations/2026XXXXXXXXXX_config.sql`:

```sql
CREATE TABLE config_settings (
    field       TEXT PRIMARY KEY,   -- Config field name, e.g. 'server_port'
    value       TEXT NOT NULL,      -- raw string, parsed by the same code every layer uses
    updated_at  INTEGER NOT NULL,
    updated_by  TEXT NOT NULL       -- 'admin-api' | 'cli'
);

-- One row per attempt to boot with a changed stored config, so a config that prevents
-- startup can be rolled back automatically instead of wedging the service.
CREATE TABLE config_generations (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at INTEGER NOT NULL,
    snapshot   TEXT NOT NULL,       -- JSON map of config_settings at this generation
    status     TEXT NOT NULL,       -- 'pending' | 'confirmed' | 'rolled_back'
    attempts   INTEGER NOT NULL DEFAULT 0
);
```

Values are stored as raw strings in the same canonical form every other layer produces, so
parsing and validation stay in `Config::from_env_provider` and nowhere else. Keying by `field`
rather than by env key matches what the admin API already speaks.

## Boot sequence

```
1. Parse argv, resolve config from cli/env/env-file/file          (as today)
2. Open the pool, run migrations                                  (as today)
3. Read config_generations: if the newest row is 'pending'
   and attempts >= 1  ->  roll back to the newest 'confirmed'
                          snapshot, mark it 'rolled_back', log loudly
   otherwise          ->  increment attempts
4. Read config_settings into a Layer
5. Re-resolve config with the stored layer inserted
   - on failure: log, discard the stored layer, continue with step 1's config
6. Rebuild anything that reads boot values, then bind and serve
7. After N seconds of successful serving, mark the generation 'confirmed'
```

Step 5's re-resolution is why the two-phase shape is unavoidable, and it has a consequence worth
stating: **everything between steps 1 and 5 uses the pre-stored config.** That is the tracing
subscriber (`log_level`, `otel_*`) and the PID file path. Those three are technically storable
but would apply one boot late, which is exactly the kind of half-truth this codebase avoids —
so `LOG_LEVEL`, `PID_FILE` and the `OTEL_*` trio should also be `Persist::Never` unless we move
subscriber init after the pool, which is a bigger change and would lose early-boot logs. That
takes the storable set from 16 to 11, still including every live field and the interesting boot
ones (`server_host`, `server_port`, `rate_limit_*`, `admin_config_write`).

Step 3's `attempts` counter is what distinguishes "first try at a new config" from "we already
tried this and did not survive". It is incremented before the risky part and only cleared by
reaching step 7.

## The restart mechanism

The proposal was a supervisor process that spawns a child and kills/restarts it on command.
Three options, and I do not think the supervisor is the right one here.

### Option A — re-exec in place (recommended)

At the end of `main`, after graceful shutdown, after the pool is closed and telemetry is
flushed, replace the process image with a fresh copy of itself:

```rust
// argv captured at startup, before anything consumed it
std::os::unix::process::CommandExt::exec(&mut Command::new(current_exe).args(argv))
```

The environment carries over automatically, so the env layer is preserved.

- **Same PID.** `docker stop`, Kubernetes, systemd and the PID file all keep working with zero
  extra code. This is the big one: the service stays a single process and stays PID 1 in a
  container, so nothing about signal handling changes.
- No second process to write, supervise, or reason about.
- Existing shutdown path is reused verbatim; the only new thing is a flag that says "exec
  instead of returning".

Cost: in-flight requests are dropped at the exec boundary. Not graceful, but neither is any
restart short of socket handoff with overlapping processes.

Details that need care: capture `args_os()` at startup rather than reconstructing from `Cli`;
close the pool before exec so WAL is flushed and no fd leaks into the new image; skip the PID
file removal on this path since the PID is unchanged; call exec from the main thread after the
tokio runtime has finished, never from inside a task.

### Option B — supervisor parent (the proposal)

A parent that spawns the real server as a child and restarts it on demand.

Genuinely better at one thing: if the child fails to start, the parent still exists and can
retry with the previous config. Option A has no such observer — the process is simply gone.

Against it: the parent becomes PID 1 in a container and inherits PID 1's obligations — reaping
orphans, and forwarding SIGTERM/SIGINT to the child, because `docker stop` signals PID 1 only.
Getting that wrong means containers that ignore `docker stop` and take the 10-second SIGKILL
every time. The PID file now names the wrong process for `captchapi reload`. And the "single
static binary, no shell, distroless or scratch" story now contains a process supervisor, which
is the part of this design most likely to have a subtle bug that only shows up in production.

Its one advantage is recoverable without it: the `config_generations` table with the `attempts`
counter gives automatic rollback on the *next* boot, which covers the same failure with a few
seconds more downtime and none of the PID 1 complexity. Combined with pre-flight checks (below)
that catch the realistic failures *before* restarting at all, the residual risk is small.

Where a supervisor would genuinely win is true zero-downtime restarts — two overlapping children
sharing a listening socket, draining the old one. That is a much larger feature and a separate
decision; if it is wanted, it should be designed as such rather than arrived at sideways.

### Option C — exit and let the orchestrator restart

Persist, then exit with a distinct code. Correct behaviour under systemd `Restart=always`,
Kubernetes, or `docker run --restart`. Zero new machinery.

Fails for a bare `cargo run` or a plain `docker run`, and in Kubernetes a non-zero exit looks
like a crash in every dashboard.

### Recommendation

Option A, with Option C available as a configuration choice (`ADMIN_RESTART_MODE=exec|exit`,
default `exec`). Reject Option B unless overlapping zero-downtime restart becomes a requirement.

## Not restarting into a broken config

Four layers, cheapest first:

1. **Validate.** Run the candidate through `Config::from_env_provider` before writing anything.
   This is the existing validation path and catches every type and range error. Reject with 400.
2. **Pre-flight the resources validation cannot see.** If `server_host`/`server_port` changed,
   attempt a bind on the new address and release it; reject with 409 if it is taken. This is the
   realistic failure — "port already in use" is precisely what someone editing `server_port`
   through a web form will hit, and it is invisible to validation.
3. **Generations with automatic rollback**, as in the boot sequence above. Covers whatever the
   first two missed.
4. **Escape hatches that need no database access:** `--ignore-stored-config` (also
   `IGNORE_STORED_CONFIG=true`), plus `captchapi config unset <field>` and
   `captchapi config clear` for offline repair.

Worth calling out: storing `admin_config_write = false` locks the console out of its own
settings. The CLI escape hatch covers it, and the console should warn before letting someone do
it.

## API surface

The durable store becomes its own resource rather than overloading the existing PATCH, which
keeps today's ephemeral-override semantics working and unbroken:

| method | path | effect |
|--------|------|--------|
| `GET` | `/api/v1/admin/config` | as today, plus `source` and `stored` per field |
| `PATCH` | `/api/v1/admin/config` | **unchanged** — ephemeral, live fields only, cleared by reload |
| `GET` | `/api/v1/admin/config/stored` | the stored layer, secrets absent by construction |
| `PUT` | `/api/v1/admin/config/stored` | store fields; 202 with `pending_restart` if any is boot-only |
| `DELETE` | `/api/v1/admin/config/stored/{field}` | remove one stored field |
| `POST` | `/api/v1/admin/restart` | graceful shutdown then re-exec; **off by default** |

`POST /restart` is a remote kill switch for the service. It should be gated by its own
parameter (`ADMIN_RESTART_ENABLED`, default false), master-key only, audit-logged, and counted
in metrics. `ADMIN_CONFIG_WRITE=false` disables the stored writes too, matching how it already
disables PATCH.

New `AppError` variants (`ConfigNotPersistable`, `RestartNotEnabled`, `AddressUnavailable`)
**will break `bindings/nodejs`**, which matches `AppError` exhaustively with no wildcard arm.
Build with `--workspace`, as CLAUDE.md already warns.

`POST /config/reload` gains a wrinkle worth being deliberate about: it clears the ephemeral
overlay, but it should *re-read* the stored layer, because the stored layer is now a source of
truth rather than an override. Reload means "re-read the sources"; the store is one.

## Console changes

- A third state per row: effective value, stored value, and where the effective one came from.
  Rows whose stored value is shadowed by CLI or environment get an explicit marker — this is the
  honesty requirement from the precedence section, and it is the main new UI concept.
- Boot rows become editable, tagged `restart to apply` instead of `restart to change`.
- A pending-restart banner listing the waiting fields, with a Restart button when the endpoint
  is enabled.
- Storable vs ephemeral needs to be visible without a manual: a per-row toggle is probably too
  fussy for 11 fields, so a single "store these changes" checkbox next to Apply is likely
  better. Worth prototyping both.

## Delivery

Three changes, each shippable and useful alone:

1. **Persistence and precedence.** Migration, `Persist` column, stored layer, two-phase boot,
   `source`/`stored` in the config API, the `/config/stored` endpoints, CLI verbs,
   `--ignore-stored-config`. After this, boot fields are editable and apply on whatever restart
   the operator already performs. No restart machinery at all.
2. **Self-restart.** Generations table, rollback, pre-flight bind check, `POST /restart`,
   re-exec, `ADMIN_RESTART_ENABLED` / `ADMIN_RESTART_MODE`.
3. **Console.** Shadow reporting, editable boot rows, pending-restart banner, restart button.

## Testing

The existing gates cover most of it, with three things needing new kinds of test:

- **Precedence and shadowing** — unit tests on the layer stack, in the style of the existing
  `handle.rs` tests, including that a `Persist::Never` field is refused and that a shadowed
  stored value is reported as shadowed.
- **Rollback** — write a `pending` generation with `attempts = 1` and an unbootable value,
  then assert the boot path rolls it back. Testable without any process machinery.
- **Re-exec** — not unit-testable. Needs an integration test that spawns the real binary,
  stores a boot field, calls restart, and re-queries the config on the new image, asserting the
  PID is unchanged. This is the one genuinely new test harness in the plan.

Plus Bruno coverage for the new endpoints, and `docs/API.md`, `CLAUDE.md` (precedence table,
schema section), `.env.example` and `captchapi.toml.example` updates.

## Open questions

1. Is losing in-flight requests on restart acceptable? If not, the answer is overlapping
   processes with socket handoff, which is a materially bigger project than any option above.
2. Should `LOG_LEVEL` / `PID_FILE` / `OTEL_*` be storable at the cost of moving subscriber
   initialization after the pool opens, and losing early-boot logs in the process?
3. Should `PUT /config/stored` optionally restart in the same request, or always leave the
   restart as a separate deliberate call?
