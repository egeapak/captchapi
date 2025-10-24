# GitHub Actions Workflows

This directory contains CI/CD workflows for automated testing.

## Workflows

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
- Generates code coverage report
- Uploads to Codecov
- Enforces 85% coverage threshold
- Duration: ~2-3 minutes

**Phase 2: Sequential API Tests**

**5. api-tests** (depends on `test` job)
- Builds the project
- Starts the server in background
- Waits for server to be ready (health check)
- Installs Bruno CLI
- Runs comprehensive API test suite (38 tests)
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
- [ ] Code coverage reporting (tarpaulin)
- [ ] Performance benchmarking
- [ ] Docker image building
- [ ] Deployment to staging
- [ ] Security scanning (cargo-audit)
- [ ] Dependency updates (dependabot)
