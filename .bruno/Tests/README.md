# Bruno Test Suite

This directory contains all test-specific endpoints, scripts, and documentation for the CaptchAPI Bruno collection.

## Directory Structure

> **Folder tree:** see [README.md](../README.md#collection-structure), which is kept in sync with
> the filesystem. Duplicating it here is what let three copies drift apart.

## Running Tests

### Quick Test (Happy Path Only)

Tests the basic workflow with all success scenarios:

```bash
# From project root
./.bruno/Tests/Scripts/test-bruno.sh

# Or from anywhere
cd /path/to/captchapi
./.bruno/Tests/Scripts/test-bruno.sh
```

**Coverage**: 5 requests, 13 tests
- Health check
- Create API key → List keys
- Create session → Get images

### Comprehensive Test Suite

Tests all endpoints with both success and failure scenarios:

```bash
# From project root
./.bruno/Tests/Scripts/test-bruno-full.sh
```

**Coverage**: 41 requests, 95 tests
- ✅ All success scenarios
- ❌ Unauthorized access
- ❌ Invalid parameters
- ❌ Not found errors
- ❌ Business logic failures

## Test Scenarios

### API Key Tests

| Test | Type | Description |
|------|------|-------------|
| Create API Key - Unauthorized | ❌ Failure | Missing master key → 401 |
| Create API Key for Testing | ✅ Success | Creates fixture for update/delete tests |
| Update API Key (Test) | ✅ Success | Updates description/active status |
| Delete API Key (Test) | ✅ Success | Deletes the test fixture |
| Delete API Key - Not Found | ❌ Failure | Non-existent key → 404 |

### Session Tests

| Test | Type | Description |
|------|------|-------------|
| Create Session - Unauthorized | ❌ Failure | Missing API key → 401 |
| Create Session - Invalid Parameters | ❌ Failure | difficulty=99 → 400 |
| Get Image - Not Found | ❌ Failure | Non-existent session → 404 |
| Validate Session - Wrong Answer | ❌ Failure | Incorrect solution (attempt 1) |
| Validate Session - Max Attempts | ❌ Failure | Attempts 2 & 3 |
| Validate Session - Session Deleted | ❌ Failure | Session auto-deleted after max attempts |

## Test vs Core Endpoints

### Core Endpoints (Main Collection)

Located in the root of the collection, these are for **normal API usage**:
- Ready to use with manual data entry
- Intended for interactive testing in Bruno GUI
- Use placeholder values (e.g., `YOUR_KEY_HASH_HERE`)

### Test Endpoints (This Directory)

Test-specific scenarios for **automated testing**:
- Designed for CI/CD and automated test runs
- Auto-populate variables via post-response scripts
- Include failure scenarios (unauthorized, invalid, not found)
- Use test fixtures (e.g., `test_key_hash` variable)

## Variable Management

Tests automatically manage these environment variables:

- `api_key` - Set by core "Create API Key"
- `session_id` - Set by core "Create Session"
- `test_key_hash` - Set by "Create API Key for Testing"

These variables are scoped to a single `bru run` execution and persist across requests within that run.

## Writing New Tests

### Failure Test Template

```javascript
meta {
  name: Test Name - Failure Scenario
  type: http
  seq: 1
}

post {
  url: {{base_url}}/api/v1/endpoint
}

headers {
  // Intentionally missing or wrong header
}

body {
  {
    "invalid_field": "wrong value"
  }
}

tests {
  test("should return error status", function() {
    expect(res.status).to.equal(400);
  });

  test("should return error message", function() {
    expect(res.body.error).to.be.a("string");
  });
}
```

### Success Test with Fixture

```javascript
meta {
  name: Test Name (Test)
  type: http
  seq: 2
}

post {
  url: {{base_url}}/api/v1/endpoint/{{test_variable}}
}

headers {
  Authorization: Bearer {{api_key}}
}

script:post-response {
  // Save for subsequent tests
  if (res.status === 201) {
    bru.setVar("new_variable", res.body.id);
  }
}

tests {
  test("should succeed", function() {
    expect(res.status).to.equal(201);
  });
}
```

## CI/CD Integration

### GitHub Actions Example

```yaml
name: API Tests

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3

      - name: Install Bruno CLI
        run: npm install -g @usebruno/cli

      - name: Start CaptchAPI
        run: cargo run &

      - name: Wait for server
        run: sleep 5

      - name: Run comprehensive tests
        run: ./.bruno/Tests/Scripts/test-bruno-full.sh
```

### Expected Output

```
📊 Execution Summary
┌───────────────┬────────────────┐
│ Status        │     ✓ PASS     │
│ Requests      │ 41 (41 Passed) │
│ Tests         │     95/95      │
│ Duration (ms) │      ~150      │
└───────────────┴────────────────┘
```

## Best Practices

1. **Separate concerns**: Keep test scenarios separate from core endpoints
2. **Use fixtures**: Create test data via "for Testing" requests
3. **Clean up**: Delete test fixtures after use
4. **Sequence matters**: Run tests in the correct order using scripts
5. **Variable scope**: Variables only persist within a single `bru run`
6. **Documentation**: Update TEST-SCENARIOS.md when adding new tests

## Troubleshooting

### Tests fail with "unauthorized"
- Ensure server is running
- Check `.env` has correct `MASTER_API_KEY`
- Verify `environments/local.bru` has matching `master_key`

### Variables not found
- Run multiple requests in a single `bru run` command
- Check post-response scripts are setting variables correctly
- Variables don't persist between separate `bru run` invocations

### Tests timeout
- Increase timeout: `bru run --timeout 30000`
- Check server is responsive
- Reduce CAPTCHA difficulty in test requests

## See Also

- **Documentation/TEST-SCENARIOS.md** - Detailed test scenario documentation
- **../README.md** - Main collection documentation
- **../../README.md** - Project documentation
