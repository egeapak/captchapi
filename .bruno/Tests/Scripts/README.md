# Bruno Test Scripts

Automated test execution scripts for CaptchAPI.

## Available Scripts

### `test-bruno.sh`

**Purpose**: Quick smoke test (happy path only)

**What it tests**:
- Health check
- Create API key → List keys
- Create session → Get images (JSON & binary)

**Coverage**: 5 requests, 13 tests

**Usage**:
```bash
# Local environment (default)
./.bruno/Tests/Scripts/test-bruno.sh

# CI environment
BRUNO_ENV=ci ./.bruno/Tests/Scripts/test-bruno.sh
```

**Duration**: ~130ms

---

### `test-bruno-full.sh`

**Purpose**: Comprehensive test suite with success and failure scenarios

**What it tests**:
- All endpoints (health, API keys, sessions)
- Success scenarios (happy paths)
- Failure scenarios (unauthorized, invalid params, not found)
- Business logic (max attempts, session deletion)

**Coverage**: 41 requests, 95 tests

**Usage**:
```bash
# Local environment (default)
./.bruno/Tests/Scripts/test-bruno-full.sh

# CI environment
BRUNO_ENV=ci ./.bruno/Tests/Scripts/test-bruno-full.sh
```

**Duration**: ~170ms

---

## Prerequisites

**Server must be running** before executing these scripts:

```bash
# Terminal 1: Start server
cargo run

# Terminal 2: Run tests
./.bruno/Tests/Scripts/test-bruno-full.sh
```

---

## Environment Variables

Scripts support environment selection via `BRUNO_ENV`:

```bash
# Use local environment (default)
./.bruno/Tests/Scripts/test-bruno-full.sh

# Use CI environment
BRUNO_ENV=ci ./.bruno/Tests/Scripts/test-bruno-full.sh

# Use production environment
BRUNO_ENV=production ./.bruno/Tests/Scripts/test-bruno-full.sh
```

### Environment Files

- **local** → `.bruno/environments/local.bru` (development)
- **ci** → `.bruno/environments/ci.bru` (GitHub Actions)
- **production** → `.bruno/environments/ci.bru` (production testing)

---

## CI/CD Integration

### GitHub Actions

The scripts are used in `.github/workflows/tests.yml`:

```yaml
- name: Run API tests
  env:
    BRUNO_ENV: ci
  run: ./.bruno/Tests/Scripts/test-bruno-full.sh
```

The workflow:
1. Builds the project
2. Starts server in background
3. Waits for health check
4. Sets `BRUNO_ENV=ci`
5. Runs comprehensive test suite
6. Uploads artifacts on failure

See `.github/workflows/README.md` for details.

---

## Output Format

### Success

```
==========================================
   CaptchAPI - Comprehensive Test Suite
==========================================

Testing all endpoints with success and failure scenarios

Using environment: local

[... test execution ...]

📊 Execution Summary
┌───────────────┬────────────────┐
│ Status        │     ✓ PASS     │
│ Requests      │ 41 (41 Passed) │
│ Tests         │     95/95      │
│ Duration (ms) │      170       │
└───────────────┴────────────────┘

==========================================
   ✓ All Tests Passed!
==========================================
```

### Failure

```
API Keys/Create API Key (401 Unauthorized) - 12 ms
[Function: AssertionError]
AssertionError: expected 401 to equal 201
    at ...

Tests
   ✕ should return 201
      expected 401 to equal 201

📊 Execution Summary
┌───────────────┬────────────────┐
│ Status        │     ✗ FAIL     │
│ Requests      │ 18 (1 Failed)  │
│ Tests         │     36/38      │
└───────────────┴────────────────┘
```

---

## Troubleshooting

### Connection Refused Errors

```
Health Check (connect ECONNREFUSED 127.0.0.1:3000)
```

**Solution**: Server is not running. Start it first:
```bash
cargo run
```

### Environment Not Found

```
Error: Environment 'xyz' not found
```

**Solution**: Check environment exists in `.bruno/environments/`:
```bash
ls .bruno/environments/
# Should show: ci.bru, local.bru
```

### Tests Fail But Server Works

**Check**:
1. Server is using correct port (3000)
2. Environment variables are set correctly
3. Master key matches between .env and environment file

**Debug**:
```bash
# Test server manually
curl http://127.0.0.1:3000/health

# Run with verbose output
cd .bruno && bru run --verbose "Health Check.bru" --env local
```

### Script Not Executable

```bash
chmod +x ./.bruno/Tests/Scripts/*.sh
```

---

## Script Internals

### How Scripts Work

1. **Change directory** to Bruno collection root
   ```bash
   cd "$(dirname "$0")/../../"
   ```

2. **Detect environment** (defaults to `local`)
   ```bash
   ENVIRONMENT="${BRUNO_ENV:-local}"
   ```

3. **Run tests** with `bru run`
   ```bash
   bru run "Health Check.bru" ... --env "$ENVIRONMENT"
   ```

### Why Single `bru run` Command?

Environment variables set by post-response scripts (e.g., `api_key`, `session_id`) only persist within a single `bru run` execution.

Running separately would fail:
```bash
# ❌ WRONG - Variables don't persist
bru run "API Keys/Create API Key.bru" --env local
bru run "Sessions/Create Session.bru" --env local  # api_key not set!

# ✅ CORRECT - Variables persist
bru run "API Keys/Create API Key.bru" "Sessions/Create Session.bru" --env local
```

---

## Adding Tests to Scripts

### To Quick Script

Edit `test-bruno.sh`:
```bash
bru run \
  "Health Check.bru" \
  "API Keys/Create API Key.bru" \
  "Your New Request.bru" \    # Add here
  --env "$ENVIRONMENT"
```

### To Comprehensive Script

Edit `test-bruno-full.sh`:
```bash
bru run \
  "Health Check.bru" \
  "Tests/Your Category/Your Test.bru" \  # Add here
  --env "$ENVIRONMENT"
```

**Order matters!** Add requests in dependency order (e.g., create before validate).

---

## Performance

### Execution Times

- **test-bruno.sh**: ~130-150ms (6 requests)
- **test-bruno-full.sh**: ~5s (41 requests)

### Optimization

Scripts are already optimized:
- Single `bru run` command (no process overhead)
- Variable reuse (no repeated API key creation)
- Efficient request ordering

---

## See Also

- **../ README.md** - Test collection overview
- **../Documentation/TEST-SCENARIOS.md** - Detailed test scenarios
- **.github/workflows/README.md** - CI/CD documentation
- **.claude/CLAUDE.md** - Project documentation and testing guide
