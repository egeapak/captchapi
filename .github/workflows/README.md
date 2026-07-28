# GitHub Actions Workflows

This directory contains CI/CD workflows for automated testing and release.

## Workflows

### `release.yml` - Tagged Release

Runs on every pushed tag matching `v*`. Publishes a multi-platform container image to
GitHub Container Registry and cuts the GitHub release.

#### Jobs

**1. validate-version**
- Rejects a tag that is not semver, so a typo cannot produce a half-tagged image
- Fails if the tag disagrees with the version in `Cargo.toml`
- Classifies the tag as stable or pre-release; everything downstream keys off that

**2. ci** — format, clippy and the full test suite, as a gate on publishing

**3. build-binaries**
- Cross-compiles static musl binaries for `x86_64` and `aarch64` with `--features otel`
- Uploads them as an artifact. This is the expensive step (two targets under full LTO), so it
  runs once and every image variant reuses the result rather than rebuilding it per variant

**4. build-and-push** — a matrix over the two image variants
- Builds the amd64 image, **runs it**, and only then pushes:
  health, `/data` volume placement, a full CAPTCHA round-trip and SIGHUP reload
- Pushes `linux/amd64` + `linux/arm64` under the tags below
- Attaches a signed build provenance attestation to the pushed digest
- Measures the image and writes the real numbers to the run summary

**5. create-release** — release notes via git-cliff, marked pre-release when the tag is one

#### Tags published

Two variants, both multi-arch, from the same binaries. For `v1.2.3`:

| variant | Dockerfile | tags |
|---------|-----------|------|
| distroless (default) | `docker/Dockerfile.multiarch` | `1.2.3`, `1.2`, `1`, `latest` |
| scratch | `docker/Dockerfile.scratch` | `scratch-1.2.3`, `scratch-1.2`, `scratch-1`, `scratch-latest` |

The variants are separated by a tag *prefix* (`flavor: prefix=…,onlatest=true`). `onlatest` is
load-bearing: without it the scratch job would publish a bare `latest` and race the distroless
job for it, and which one won would come down to scheduling.

For a pre-release such as `v2.0.0-rc.1`: **only** `2.0.0-rc.1` and `scratch-2.0.0-rc.1`. The
moving tags are left pointing at the last stable release, so nobody tracking `latest` or `1` is
upgraded onto a release candidate.

`fail-fast` is off. If one variant cannot be published the other still should be — a registry
holding one half of a release is worse than two red jobs.

#### Ordering

The smoke test runs *before* the push. It used to run after, which meant an image that failed
it had already been published — including under `latest`, where a default `docker pull` picks
it up. Keep the order.

---

### `ci.yml` - Continuous Integration

Runs on every push and pull request to `main`/`master` branches.

#### Jobs

**Phase 1: Parallel Quality Checks**

**1. format**
- Checks code formatting (`cargo fmt --check`)
- Duration: ~30 seconds

**2. clippy**
- Runs Clippy linter (`cargo clippy`)
- Fails on any warnings
- Duration: ~1-2 minutes

**3. test**
- Runs all Rust unit and integration tests
- Duration: ~2-3 minutes

**4. coverage**
- Runs the suite under `cargo llvm-cov` + nextest, uploads to Codecov, and
  enforces both a project floor and a **patch** threshold
- Duration: ~2-3 minutes

See [Code coverage](#code-coverage) below — the job is shaped by two constraints
that are not obvious from reading it.

**Phase 2: Sequential API Tests**

**5. api-tests** (depends on `test` job)
- Builds the project
- Starts the server in background
- Waits for server to be ready (health check)
- Installs Bruno CLI
- Runs comprehensive API test suite (95 tests)
- Uploads artifacts on failure
- Always stops server
- Duration: ~3-4 minutes

#### Workflow Benefits

**Better than separate workflows:**
- ✅ Single workflow to manage
- ✅ Unified status in Actions tab
- ✅ Shared caching strategy
- ✅ Better Rust actions (dtolnay, Swatinem)
- ✅ Existing coverage integration preserved

**Better than all-parallel:**
- ✅ API tests skip if Rust tests fail
- ✅ Saves ~50% time on failures
- ✅ Faster feedback on broken code

**Better than all-sequential:**
- ✅ Format, clippy, test, coverage run in parallel
- ✅ Only API tests are sequential (depends on test)
- ✅ Optimal balance of speed and efficiency

---

## Code coverage

Coverage is measured once per run and reported three ways: to Codecov, to the
run summary and pull-request comment, and as a downloadable HTML report
(`coverage-report` artifact, 14 days).

### Two gates, and which one to require

| gate | where | asks |
|---|---|---|
| **project** | `Code Coverage` job, `codecov/project` | is the codebase still above 85%, and did this change drop it more than 1%? |
| **patch** | `Code Coverage` job, `codecov/patch` | of the lines *this pull request wrote*, how many are tested? Floor 70%. |

**Patch is the one that matters.** At ~91% over ~11,900 instrumented lines, a
pull request can add forty untested lines and move project coverage by less than
a tenth of a point. Project coverage is a ratchet against slow decay; patch
coverage is what stops untested code arriving.

Thresholds live in two places that must agree: `MIN_PROJECT_COVERAGE` /
`MIN_PATCH_COVERAGE` in `ci.yml`, and the targets in `.codecov.yml`.

### Enabling "restrict code coverage" (required status checks)

The `Code Coverage` job computes both numbers itself, from `lcov.info`, via
`scripts/coverage-report.py`. It needs no third-party service, so it can be
made a required check immediately:

> Settings → Branches → branch protection rule for `master` → *Require status
> checks to pass before merging* → add **`Code Coverage`**.

To additionally require Codecov's own `codecov/project` and `codecov/patch`
checks, the repository must first be **activated** on Codecov. Until it is,
those checks never post, and a required check that never posts leaves every
pull request unmergeable rather than merely red.

- Activate at <https://app.codecov.io/github/egeapak/captchapi> and confirm the
  Codecov GitHub App is installed with access to this repository.
- Verify with `curl -s https://api.codecov.io/api/v2/github/egeapak/repos/captchapi/`
  — `"activated"` must be `true`, and `updatestamp` must be recent.
- The `Verify Codecov processed the report` step polls that same API after each
  upload and emits a warning naming this cause when a report never lands. It is
  a warning, not a failure, because processing is asynchronous.

The failure mode this guards against is silent: an un-activated repository
accepts every upload, returns "Upload queued for processing complete", and then
drops it. The CI job goes green and the dashboard quietly serves a months-old
report.

### Why the job depends on nothing

`coverage` deliberately has no `needs:`. A job behind `needs: test` is
**skipped** when an upstream job fails, and a skipped required check reports
nothing at all — branch protection then blocks the pull request on a status that
will never arrive. It also runs the full suite itself, so waiting saves nothing.

For the same reason it uses `fetch-depth: 0`: both Codecov's base comparison and
the patch gate's `git diff base...head` need the base commit, which a shallow
clone does not have.

### Running it locally

```bash
just coverage                # against origin/master, same thresholds as CI
just coverage HEAD~1         # against the previous commit
just coverage ""             # project coverage only, no patch gate
```

`src/main.rs` is exempt from the *patch* gate only: its startup path is
exercised by `tests/cli_smoke_test.rs` and the Bruno suite, but both drive a
subprocess, so in-process llvm-cov instrumentation never sees it. It still
counts against project coverage.

---

#### Environment Configuration

The workflow uses a CI-specific environment (`.bruno/environments/ci.bru`):
- Server: `http://127.0.0.1:3000`
- Master key: `ci-test-master-key-do-not-use-in-production`
- API key: Auto-generated during test execution

#### Artifacts

API test results are uploaded as workflow artifacts for review when tests fail.

## Running Tests Locally

### Using the CI Environment

You can run tests locally using the CI environment:

```bash
# Start server
cargo run

# Run tests with CI environment
BRUNO_ENV=ci ./.bruno/Tests/Scripts/test-bruno-full.sh
```

### Default (Local) Environment

Normal local testing uses the `local` environment:

```bash
# Start server
cargo run

# Run tests (uses local environment by default)
./.bruno/Tests/Scripts/test-bruno-full.sh
```

## Workflow Features

### Caching

The workflow caches:
- Cargo registry
- Cargo git index
- Build artifacts (`target/` directory)

This speeds up subsequent runs significantly.

### Health Check

Before running API tests, the workflow:
1. Starts the server in the background
2. Waits up to 60 seconds for server to be ready
3. Polls `/health` endpoint every 2 seconds
4. Fails if server doesn't start within timeout

### Cleanup

The server is always stopped after tests complete, even if tests fail.

## Viewing Results

### In GitHub UI

1. Go to **Actions** tab
2. Click on the workflow run
3. View job results:
   - Green checkmark: All tests passed
   - Red X: Tests failed
4. Click job name to see detailed logs
5. Download artifacts to review test results

### Test Failures

When API tests fail:
1. Check the "Run API tests" step logs
2. Download the `api-test-results` artifact
3. Review error messages
4. Common issues:
   - Server failed to start
   - Database initialization errors
   - Authentication failures

## Status Badge

Add this badge to your README.md to show test status:

```markdown
![Tests](https://github.com/YOUR_USERNAME/captchapi/workflows/Tests/badge.svg)
```

Replace `YOUR_USERNAME` with your GitHub username.

## Local Testing with Act

You can test workflows locally using [act](https://github.com/nektos/act):

```bash
# Install act
brew install act  # macOS
# or
curl https://raw.githubusercontent.com/nektos/act/master/install.sh | sudo bash

# Run workflows locally
act push

# Run specific job
act -j rust-tests
act -j api-tests
```

## Troubleshooting

### Server Won't Start

Check logs for:
- Database connection errors
- Port already in use (3000)
- Missing migrations

### API Tests Timeout

Increase wait timeout in workflow:
```yaml
- name: Wait for server to be ready
  run: |
    max_attempts=60  # Increase from 30
```

### Cargo Cache Issues

Clear cache by:
1. Go to repository Settings
2. Click Actions → Caches
3. Delete all caches
4. Re-run workflow

## Environment Variables

The workflow sets:
- `CARGO_TERM_COLOR=always` - Colored cargo output
- `RUST_BACKTRACE=1` - Full backtraces on panics
- `BRUNO_ENV=ci` - Use CI environment (set in test scripts)

## Future Enhancements

Potential improvements:
- [ ] Performance benchmarking
- [ ] Deployment to staging
- [ ] Dependency updates (dependabot)
- [ ] Build the container image on pull requests too — today a Dockerfile regression is
      only caught by `release.yml`, i.e. at tag time, when it is most expensive to fix
- [ ] Attach the static binaries to the GitHub release, for users not deploying containers

Done since this list was written: code coverage (`coverage` job — project floor,
patch gate, Codecov export, HTML artifact; see [Code coverage](#code-coverage)),
security scanning (`security-audit` job, `cargo audit`), and container image building
(`release.yml`).

Still outstanding on coverage, and worth doing in this order:
- [ ] **Activate the repository on Codecov.** Everything else here is wired; the
      `codecov/*` checks cannot be required until this is done by hand.
- [ ] `src/main.rs` (0%, 228 lines) and `src/tasks/log_filter.rs` (0%, 24 lines)
      are the two largest gaps. Both are startup wiring reachable only from a
      real process, so closing them means extracting the logic rather than
      writing more tests against it.
- [ ] `src/services/captcha/generator.rs` sits at 63% — the largest gap in code
      that *is* in-process testable, mostly the higher-difficulty deformation
      branches.
