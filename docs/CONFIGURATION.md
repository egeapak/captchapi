# Configuration

Operator reference for every CaptchAPI setting: where values can come from, which of them can
change without a restart, which survive one, and how to recover from one that prevents startup.

For *why* the system is built this way, see [CLI_CONFIG_DESIGN.md](CLI_CONFIG_DESIGN.md) and
[CONFIG_PERSISTENCE_DESIGN.md](CONFIG_PERSISTENCE_DESIGN.md). For the HTTP endpoints that read
and write configuration, see [API.md](API.md#admin-endpoints).

## Precedence

Seven layers. Highest wins:

```
admin API > command line > environment > SQLite store > env file (.env) > config file > default
```

- **admin API** — `PATCH /api/v1/admin/config`. Ephemeral: cleared by the next reload.
- **command line** — flags, see [Command line](#command-line).
- **environment** — the `Variable` column in the tables below.
- **SQLite store** — settings written through `PUT /api/v1/admin/config/stored`, which survive a
  restart. See [Persisted settings](#persisted-settings).
- **env file** — `.env` by default; `--env-file` to relocate, `--no-env-file` to skip.
- **config file** — TOML, `-c/--config`. See `captchapi.toml.example`.

An environment-only deployment is unaffected by any of the others existing.

To see the resolved value of every setting and which layer supplied it:

```bash
captchapi config show
# server_port             = 8080                  [cli]
# database_url            = sqlite:./data/x.db    [env]
# captcha_compression     = 75                    [file: captchapi.toml]
# max_validation_attempts = 3                     [default]
# api_key_salt            = <redacted, 32 bytes>  [env]
```

### Pinned fields

A field whose effective value came from the **command line** or the **process environment** is
refused by `PATCH /config` with `409 config_pinned`. Those two layers are fixed for the life of
the process, so an override would work only until the next reload discarded it, and could never
be made durable without a restart — accepting it would put the running server out of step with
the deployment that declared it.

File-supplied values (env file, TOML) stay patchable, because a file can be edited and re-read.
`GET /config` reports `source` and `editable` per field.

## Reloadable vs boot-only

**Reloadable** settings are adopted by the running process. Everything else is captured at
startup by the listener, the connection pool, the middleware or the rate limiter; a reload
*reports* that those changed and tells you a restart is needed, rather than silently pretending
to apply them.

Reload is triggered three ways:

```bash
captchapi reload                     # or: kill -HUP $(cat data/captchapi.pid)
docker kill -s HUP <container>       # same thing inside the image
curl -X POST .../api/v1/admin/config/reload -H "Authorization: Bearer $MASTER_KEY"
```

A reload that fails to resolve is logged and discarded; the running server keeps its current
configuration and is never taken down by a bad edit.

---

## Server

| Variable | Flag | Description | Default | Reload |
|---|---|---|---|---|
| `SERVER_HOST` | `-H`, `--host` | Address to bind the HTTP listener to | `0.0.0.0` | boot |
| `SERVER_PORT` | `-p`, `--port` | Port to listen on | `3000` | boot |
| `PID_FILE` | `--pid-file` | Where to write the process ID, so `captchapi reload` can find the server | `./data/captchapi.pid` | boot |

## Database

| Variable | Flag | Description | Default | Reload |
|---|---|---|---|---|
| `DATABASE_URL` | `-d`, `--database-url` | SQLite connection string (must start with `sqlite:`) | `sqlite:./data/captchapi.db` | boot |
| `DATABASE_MAX_CONNECTIONS` | `--database-max-connections` | Connection pool size | `5` | boot |

## Security

All four are **required to be at least 16 bytes**, are redacted by `config show` and the admin
API, and cannot be set in the TOML config file at all — so a config file is safe to commit.

| Variable | Flag | Description | Default | Reload |
|---|---|---|---|---|
| `API_KEY_SALT` | `--api-key-salt-file` | Salt for API key hashing | — **required** | boot |
| `MASTER_API_KEY` | `--master-api-key-file` | Master admin key | — **required** | boot |
| `SOLUTION_HASH_SECRET` | `--solution-hash-secret-file` | Key for hashing CAPTCHA solutions | `API_KEY_SALT` | boot |
| `IMAGE_ENCRYPTION_SECRET` | `--image-encryption-secret-file` | Key for encrypting stored images | `API_KEY_SALT` | boot |

The flags take a **path, not the secret itself** — a flag value would land in `ps`, shell history
and `docker inspect`. A file is also what Docker and Kubernetes secrets provide:

```bash
captchapi --api-key-salt-file   /run/secrets/salt \
          --master-api-key-file /run/secrets/master_key
```

> **Rotating `SOLUTION_HASH_SECRET` or `IMAGE_ENCRYPTION_SECRET` invalidates every stored
> session.** Existing solution hashes stop matching and stored images stop decrypting. Like
> `API_KEY_SALT` they apply at startup only; a reload reports the change rather than applying it.

## CAPTCHA

All reloadable — a reload applies them with no restart and no dropped sessions.

| Variable | Flag | Description | Default | Reload |
|---|---|---|---|---|
| `DEFAULT_SESSION_TTL_SECONDS` | `--default-session-ttl` | Default session lifetime, seconds | `300` | **live** |
| `MAX_SESSION_TTL_SECONDS` | `--max-session-ttl` | Maximum lifetime a client may request, seconds | `3600` | **live** |
| `MAX_VALIDATION_ATTEMPTS` | `--max-validation-attempts` | Failed attempts before a session is destroyed | `3` | **live** |
| `CAPTCHA_COMPRESSION` | `--captcha-compression` | JPEG quality, 1–100 (outside the range is clamped) | `40` | **live** |

Per-request `length` and `difficulty` are chosen by the API caller, not configured here — see
[CAPTCHA-TUNING.md](CAPTCHA-TUNING.md) for the measured basis of the defaults.

## Rate limiting

| Variable | Flag | Description | Default | Reload |
|---|---|---|---|---|
| `RATE_LIMIT_REQUESTS_PER_SECOND` | `--rate-limit-rps` | Sustained request rate per client IP | `2` | boot |
| `RATE_LIMIT_BURST_SIZE` | `--rate-limit-burst` | Burst capacity per client IP | `10` | boot |
| `RATE_LIMIT_REVERSE_PROXY` | `--rate-limit-reverse-proxy` | Read the client IP from proxy headers | `false` | boot |

> **Only enable `RATE_LIMIT_REVERSE_PROXY` behind a proxy you trust.** Clients can otherwise
> spoof the header and evade the limit entirely.

## Background tasks

| Variable | Flag | Description | Default | Reload |
|---|---|---|---|---|
| `CLEANUP_INTERVAL_SECONDS` | `--cleanup-interval` | How often expired sessions are removed, seconds | `60` | **live** |

## Admin

| Variable | Flag | Description | Default | Reload |
|---|---|---|---|---|
| `ADMIN_CONFIG_WRITE` | `--admin-config-write` | Allow `PATCH /api/v1/admin/config` to change settings at runtime | `true` | boot |
| `ADMIN_RESTART_ENABLED` | `--admin-restart-enabled` | Allow `POST /api/v1/admin/restart` to restart the server | `false` | boot |
| `IGNORE_STORED_CONFIG` | `--ignore-stored-config` | Start without consulting the SQLite config store | `false` | boot |

Set `ADMIN_CONFIG_WRITE=false` for deployments that want file-driven reload but no remote
mutation: `GET /admin/config` and `POST /admin/config/reload` keep working and `PATCH` returns
`403`.

`POST /api/v1/admin/restart` is off by default. It restarts in place with `execve`, so the PID
never changes and `docker stop`, Kubernetes, systemd and the PID file keep working. It dry-runs
the new configuration and test-binds the new address first, because "port already in use" is
invisible to validation and a restart into it leaves the service *down* rather than unchanged.

## Logging

| Variable | Flag | Description | Default | Reload |
|---|---|---|---|---|
| `RUST_LOG` | `--log-level` | Tracing filter directives | `captchapi=debug,tower_http=debug` | **live** |

Directives are `target=level` pairs or a bare level, comma separated — `captchapi=info,tower_http=warn`,
or just `debug`. Per-span field filtering (`[span{field=value}]=level`) is **not** supported: the
filter is built on `tracing_subscriber::filter::Targets` rather than `EnvFilter`, to keep the
`regex` engine out of the binary.

## OpenTelemetry

Trace and metric export sits behind the `otel` cargo feature, because the OTLP exporter pulls in
a full HTTP client worth roughly 700 KB of binary:

```bash
cargo build --release --features otel
```

**The published Docker images are built with `otel` enabled**, so these work out of the box
there. A plain `cargo build` omits it, and such a binary warns on startup if telemetry is enabled
rather than dropping traces silently.

| Variable | Flag | Description | Default | Reload |
|---|---|---|---|---|
| `OTEL_ENABLED` | `--otel` | Export traces and metrics over OTLP | `false` | boot |
| `OTEL_EXPORTER_OTLP_ENDPOINT` | `--otel-endpoint` | OTLP collector endpoint | `http://localhost:4318` | boot |
| `OTEL_SERVICE_NAME` | `--otel-service-name` | Service name reported on exported telemetry | `captchapi` | boot |

Telemetry is **pushed** over OTLP. Nothing scrapes this service and there is no metrics endpoint
to poll; to use Prometheus, have the collector re-expose them.

`OTEL_EXPORTER_OTLP_ENDPOINT` takes a **complete URL**, and the signal path is appended for you.

---

## Persisted settings

`PUT /api/v1/admin/config/stored` writes settings to a `config_settings` table in the database,
which are applied as a configuration layer at startup and therefore **survive a restart**. This
is how a setting captured at boot — the port, the rate limits — is changed durably without
editing a file inside a container image.

Fourteen of the twenty-three settings may be stored. Nine are refused with
`400 config_not_persistable`, for exactly three reasons:

| Not persistable | Why |
|---|---|
| `API_KEY_SALT`, `MASTER_API_KEY`, `SOLUTION_HASH_SECRET`, `IMAGE_ENCRYPTION_SECRET` | secrets — never written to the database |
| `DATABASE_URL`, `DATABASE_MAX_CONNECTIONS` | needed to *open* the database the store lives in |
| `OTEL_ENABLED`, `OTEL_EXPORTER_OTLP_ENDPOINT`, `OTEL_SERVICE_NAME` | consumed before the store is read |

### Automatic rollback

Every write snapshots the table as a `pending` generation. A boot that reads it increments
`attempts`; a process that serves for 30 seconds marks it `confirmed`; a boot that finds a
`pending` generation *already attempted* restores the newest confirmed snapshot. A configuration
that stops the service therefore un-applies itself on the next start.

A configuration that fails to *resolve* is skipped so the service stays up, and its generation is
deliberately left unconfirmed — confirming it would record settings that do not work as the ones
every later rollback restores to.

### Recovery

**The distroless and scratch images have no shell and no `sqlite3`**, so these are the only way
to undo a stored value that prevents startup:

```bash
captchapi config unset server_port     # remove one setting from the store
captchapi config clear                 # remove every setting from the store
captchapi --ignore-stored-config       # start once without consulting the store at all
```

`--ignore-stored-config` (also `IGNORE_STORED_CONFIG=true`) is checked *before* the generation
bookkeeping runs: a start told to ignore the store does not write to it.

The command line and the environment outrank the store precisely so that `captchapi --port 3000`
recovers a bad stored port without any of the above.

---

## Command line

```
captchapi                          Start the server (default verb)
captchapi config show              Print the effective configuration with provenance
captchapi config check             Validate the configuration and exit
captchapi config unset <FIELD>     Remove one setting from the SQLite store
captchapi config clear             Remove every setting from the SQLite store
captchapi reload [--pid N]         Tell a running server to re-read its configuration
captchapi --help                   Full flag list
```

Every setting has a flag named after its variable — `--port`, `--captcha-compression`,
`--rate-limit-rps` — plus `-c/--config`, `--env-file`, `--no-env-file` and
`--ignore-stored-config`.

Exit codes are `0` for success, `1` for a runtime error and `2` for a usage or configuration
error, so `config check` works as a deployment-pipeline gate:

```bash
captchapi config check -c /etc/captchapi/captchapi.toml || exit 1
```

## Adding a setting

One row in `PARAMS` (`src/config/params.rs`) and one field on `Config` (`src/config/mod.rs`).
The flag, help text, TOML key, provenance reporting and the reloadable/boot-only split are all
derived from that table, and tests enforce that the two stay in sync — including that every
declared default matches what the code actually produces.
