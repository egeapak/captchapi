# CI/CD Documentation

Complete guide to the CaptchAPI continuous integration and deployment setup.

## Overview

The project uses GitHub Actions to automatically test every push and pull request. Tests run in parallel for faster feedback.

## Workflow File

**Location**: `.github/workflows/ci.yml`

**Triggers**:
- Push to `main` or `master` branch
- Pull requests to `main` or `master` branch

## Jobs

### Phase 1: Parallel Quality Checks

**1. `format` (~30 seconds)**

Validates code formatting.

**Steps**:
- Checkout code
- Install Rust toolchain with rustfmt
- Cache dependencies (Swatinem/rust-cache)
- Check formatting: `cargo fmt --all -- --check`

**Fails if**: Code is not properly formatted

---

**2. `clippy` (~1-2 minutes)**

Runs Rust linter.

**Steps**:
- Checkout code
- Install Rust toolchain with clippy
- Cache dependencies
- Run linter: `cargo clippy --all-targets --all-features -- -D warnings`

**Fails if**: Any clippy warnings exist

---

**3. `test` (~2-3 minutes)**

Runs all Rust tests.

**Steps**:
- Checkout code
- Install Rust toolchain
- Cache dependencies
- Run tests: `cargo test --all-features --workspace`

**Fails if**: Any test fails

---

**4. `coverage` (~2-3 minutes)**

Generates and validates code coverage.

**Steps**:
- Checkout code
- Install Rust toolchain
- Cache dependencies
- Install cargo-llvm-cov
- Generate coverage report
- Upload to Codecov
- Enforce 85% coverage threshold

**Fails if**: Coverage drops below 85%

---

### Phase 2: API Tests (Sequential)

**5. `api-tests` (~3-4 minutes)**

**Depends on**: `test` job must pass first

Validates HTTP API interface.

**Steps**:
1. Checkout code
2. Install Rust toolchain
3. Cache dependencies
4. Create `.env` file with test credentials
5. Build project: `cargo build --release`
6. Start server in background
7. Wait for server ready (polls `/health` endpoint, 60s timeout)
8. Install Node.js
9. Install Bruno CLI: `npm install -g @usebruno/cli`
10. Run API tests: `./.bruno/Tests/Scripts/test-bruno-full.sh`
11. Stop server (always runs, even on failure)
12. Upload test artifacts (on failure)

**Fails if**:
- Server won't start
- Server doesn't become ready within 60s
- Any of the 95 API tests fail

---

## Execution Order

Jobs execute in an **optimized mixed strategy**:

```
┌─ format   (parallel) ──┐
├─ clippy   (parallel) ──┤
├─ test     (parallel) ──┼─ All must complete
└─ coverage (parallel) ──┘
         ↓
    test job passes
         ↓
    api-tests (sequential, depends on test)
```

**Rationale**:
- Phase 1 jobs run in parallel for speed
- API tests only run if `test` job passes (most common failure point)
- If basic tests fail, API tests are skipped
- **Saves ~50% CI time** on test failures
- Coverage can run in parallel (independent)

## Environment Configuration

### CI Environment

**File**: `.bruno/environments/ci.bru`

**Variables**:
```
base_url: http://127.0.0.1:3000
master_key: ci-test-master-key-do-not-use-in-production
api_key: YOUR_API_KEY_HERE (auto-generated during tests)
session_id: YOUR_SESSION_ID_HERE (auto-generated during tests)
test_key_hash: YOUR_KEY_HASH_HERE (auto-generated during tests)
```

**Important**: These credentials are for testing only. Never use in production!

### .env File (CI)

Created automatically by workflow:
```bash
SERVER_HOST=127.0.0.1
SERVER_PORT=3000
DATABASE_URL=sqlite:./data/captchapi.db
API_KEY_SALT=ci-test-salt-do-not-use-in-production
MASTER_API_KEY=ci-test-master-key-do-not-use-in-production
DEFAULT_SESSION_TTL_SECONDS=300
MAX_SESSION_TTL_SECONDS=3600
MAX_VALIDATION_ATTEMPTS=3
CLEANUP_INTERVAL_SECONDS=60
```

## Performance Optimizations

### Caching

The workflow caches three directories to speed up builds:
- `~/.cargo/registry` - Downloaded crates
- `~/.cargo/git` - Git dependencies
- `target/` - Compiled artifacts

**First run**: ~4-6 minutes
**Cached runs**: ~1-2 minutes

### Parallel Execution

`rust-tests` and `api-tests` run in parallel, saving ~3 minutes per workflow.

### Release Build

API tests use `--release` mode for faster server startup:
- Debug build: ~5-10s startup
- Release build: ~1-2s startup

## Viewing Results

### GitHub UI

1. Go to repository on GitHub
2. Click **Actions** tab
3. Click on a workflow run
4. View job status:
   - ✅ Green: All passed
   - ❌ Red: Failures
   - 🟡 Yellow: Running
5. Click job name for detailed logs

### Status Badge

Add to your README.md:
```markdown
![Tests](https://github.com/YOUR_USERNAME/captchapi/workflows/Tests/badge.svg)
```

Shows current test status in your repository.

## Artifacts

When API tests fail, the workflow uploads:
- Test results
- Server logs
- Any generated files

**To download**:
1. Go to failed workflow run
2. Scroll to bottom
3. Click "api-test-results" artifact
4. Extract and review files

## Local Workflow Testing

### Using Act

Test workflows locally before pushing:

```bash
# Install act
brew install act  # macOS
# or download from: https://github.com/nektos/act

# Run entire workflow
act push

# Run specific job
act -j rust-tests
act -j api-tests

# Run with secrets
act push --secret-file .secrets
```

**Note**: The current background Bash shows an `act` command already running!

## Troubleshooting

### Server Won't Start in CI

**Symptoms**:
- "Server failed to start after 30 attempts"
- Health check timeouts

**Solutions**:
1. Check migrations are in `migrations/` folder
2. Verify `.env` creation step
3. Check for port conflicts
4. Increase timeout in workflow

### API Tests Fail in CI But Pass Locally

**Common causes**:
- Environment variable mismatch
- Different master key in CI vs local
- Timing issues (server not fully ready)

**Debug**:
1. Download workflow artifacts
2. Check error messages
3. Run locally with CI environment:
   ```bash
   BRUNO_ENV=ci ./.bruno/Tests/Scripts/test-bruno-full.sh
   ```

### Cache Issues

**Symptoms**:
- Old dependencies used
- Stale build artifacts

**Solution**:
1. Go to Settings → Actions → Caches
2. Delete all caches
3. Re-run workflow

### Rust Tests Pass But API Tests Fail

This is expected behavior! Rust tests validate internal logic, but:
- HTTP routing might be wrong
- Request/response formats might be incorrect
- Authentication middleware might fail

**This is why both test suites are required!**

## Workflow Maintenance

### Updating Test Scripts

When adding new tests:
1. Add `.bru` files to `.bruno/Tests/`
2. Update `test-bruno-full.sh` to include new files
3. Test locally first
4. Push and verify in CI

### Changing Environment

To add staging environment:
1. Create `.bruno/environments/staging.bru`
2. Update `.bruno/.gitignore` to include it
3. Add workflow job for staging tests
4. Update documentation

### Modifying Timeout

If tests need more time:

```yaml
- name: Wait for server to be ready
  run: |
    max_attempts=60  # Increase from 30
    # ... rest of script
```

## Security Considerations

### Test Credentials

**CI credentials are public** in the repository:
- Master key: `ci-test-master-key-do-not-use-in-production`
- API salt: `ci-test-salt-do-not-use-in-production`

**Never use these in production!**

### Database

CI uses ephemeral SQLite databases:
- Created during workflow
- Destroyed after workflow completes
- No persistent data

### Server Access

CI server only binds to `127.0.0.1:3000`:
- Not accessible from outside
- No security risk
- Isolated per workflow run

## Cost Optimization

### GitHub Actions Minutes

Free tier includes:
- 2,000 minutes/month for private repos
- Unlimited for public repos

### Current Usage

Each workflow run:
- rust-tests: ~3 minutes
- api-tests: ~4 minutes
- Total: ~7 minutes (parallel execution)

**Estimated runs/month**:
- ~285 workflow runs (free tier)
- More for public repos (unlimited)

### Reducing Costs

If needed:
1. Skip API tests for draft PRs
2. Reduce cache scope
3. Use self-hosted runners
4. Run tests only on specific paths

## Best Practices

### Pull Requests

✅ **Do**:
- Wait for CI to pass before merging
- Review workflow logs for warnings
- Test locally before pushing

❌ **Don't**:
- Merge with failing tests
- Ignore CI warnings
- Push without running local tests

### Workflow Updates

✅ **Do**:
- Test with `act` locally first
- Update documentation when changing workflow
- Keep test credentials in separate environment file

❌ **Don't**:
- Hardcode credentials in workflow
- Skip validation steps
- Increase timeouts unnecessarily

## Monitoring

### Workflow Status

Monitor in GitHub:
- Actions tab shows recent runs
- Email notifications on failures
- Status badge on README

### Performance Tracking

Track over time:
- Test execution duration
- Cache hit rate
- Failure frequency

## Related Documentation

- `.github/workflows/README.md` - Workflow overview
- `.bruno/Tests/Scripts/README.md` - Test script documentation
- `.bruno/Tests/README.md` - Test collection documentation
- `.claude/CLAUDE.md` - Project documentation and testing guide

---

**Last Updated**: 2025-10-24
**Workflow Version**: 1.0
