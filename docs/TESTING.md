# CaptchAPI - Testing Documentation

Comprehensive guide to testing the CaptchAPI service.

## Test Coverage Summary

**Total Tests: 40**
- Unit Tests: 22
- Integration Tests: 17
- Migration Tests: 1

**Coverage Areas:**
- ✅ API key hashing and authentication
- ✅ CAPTCHA generation with various parameters
- ✅ Session lifecycle (create, retrieve, validate, delete)
- ✅ Binary image endpoint with proper headers
- ✅ API key management (CRUD operations)
- ✅ Master key authentication
- ✅ Error handling and edge cases
- ✅ Database migrations
- ✅ JPEG image validation

---

## Running Tests

### Run All Tests

```bash
# Run all tests
cargo test

# Run with verbose output
cargo test -- --nocapture

# Run with logging enabled
RUST_LOG=debug cargo test -- --nocapture
```

### Run Specific Test Suites

```bash
# Unit tests only
cargo test --lib

# Integration tests only
cargo test --test sessions_test
cargo test --test api_keys_test

# Migration tests
cargo test --test test_migration

# Single specific test
cargo test test_create_session_with_auth_succeeds
```

### Test Output

```
running 40 tests
test result: ok. 40 passed; 0 failed
```

---

## Test Structure

### Directory Layout

```
captchapi/
├── src/
│   └── services/
│       ├── auth.rs       # Unit tests inline
│       └── captcha.rs    # Unit tests inline
└── tests/
    ├── common/
    │   └── mod.rs        # Shared test utilities
    ├── api_keys_test.rs  # API key management integration tests
    ├── sessions_test.rs  # Session API integration tests
    └── test_migration.rs # Migration verification
```

---

## Unit Tests (22 tests)

### Auth Service Tests (4 tests)

Location: `src/services/auth.rs`

**Tests:**
1. `test_hash_api_key_produces_consistent_hash` - Same key produces same hash
2. `test_hash_api_key_different_keys_different_hashes` - Different keys produce different hashes
3. `test_hash_api_key_different_salts_different_hashes` - Salt affects hash output
4. `test_hash_api_key_empty_key` - Empty key handled correctly

**Example:**
```bash
cargo test --lib auth::tests
```

---

### Captcha Service Tests (11 tests)

Location: `src/services/captcha.rs`

**Tests:**
1. `test_generate_with_custom_text` - Custom text CAPTCHA generation with JPEG validation
2. `test_generate_with_random_text` - Random text generation with format validation
3. `test_generate_with_different_parameters` - Parameter variations produce different images
4. `test_generate_random_text_length` - Text length configuration
5. `test_generate_random_text_is_random` - Randomness verification
6. `test_generate_returns_jpeg_bytes` - Direct JPEG byte generation
7. `test_image_to_jpeg_bytes` - DynamicImage to JPEG conversion

**What's Validated:**
- JPEG signature (0xFF 0xD8 0xFF)
- Image data size (> 1000 bytes)
- Alphanumeric text generation
- Parameter handling

**Example:**
```bash
cargo test --lib captcha::tests
```

---

## Integration Tests (17 tests)

### Sessions API Tests (9 tests)

Location: `tests/sessions_test.rs`

**Tests:**
1. `test_health_check` - Health endpoint returns correct status
2. `test_create_session_without_auth_fails` - Unauthorized access rejected
3. `test_create_session_with_auth_succeeds` - Session creation with valid API key
4. `test_complete_session_flow` - Full lifecycle: create → get image → validate → verify deletion
5. `test_validate_session_with_wrong_solution` - Failed validation tracked
6. `test_delete_session` - Manual session deletion
7. `test_create_session_with_invalid_parameters` - Parameter validation
8. `test_get_nonexistent_session` - 404 for non-existent sessions
9. `test_get_binary_image` - Binary JPEG endpoint with headers validation

**Key Validations:**
- HTTP status codes (201 Created, 200 OK, 404 Not Found, 401 Unauthorized)
- JSON response structure
- Base64 data URI format
- Binary JPEG signature and headers
- Session expiration behavior
- Attempt counting

**Example:**
```bash
cargo test --test sessions_test
cargo test test_complete_session_flow -- --nocapture
```

---

### API Keys Tests (8 tests)

Location: `tests/api_keys_test.rs`

**Tests:**
1. `test_create_api_key_without_master_key_fails` - Unauthorized access rejected
2. `test_create_api_key_with_master_key_succeeds` - API key creation
3. `test_list_api_keys` - Listing all keys
4. `test_update_api_key` - Update description and status
5. `test_delete_api_key` - Delete operation
6. `test_deactivated_api_key_cannot_access_sessions` - Deactivation enforcement
7. `test_create_and_use_api_key_end_to_end` - Create key → use it to create session
8. `test_update_nonexistent_api_key` - 404 handling

**Key Validations:**
- Master key authentication
- API key generation (32 chars)
- Key deactivation enforcement
- CRUD operations
- End-to-end workflows

**Example:**
```bash
cargo test --test api_keys_test
cargo test test_create_and_use_api_key_end_to_end -- --nocapture
```

---

## Test Utilities

### TestApp Helper

Location: `tests/common/mod.rs`

The `TestApp` struct provides a complete test environment:

```rust
pub struct TestApp {
    pub storage: StorageService,
    pub auth_service: Arc<AuthService>,
    pub api_key: String,        // Pre-created test API key
    pub master_key: String,     // Test master key
}
```

**Features:**
- Creates isolated in-memory SQLite database per test instance
- Runs migrations automatically
- Pre-creates a test API key for immediate use
- Builds Axum app with all routes configured

**Usage:**
```rust
#[tokio::test]
async fn my_test() {
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Use test_app.api_key for authentication
    let response = server
        .post("/api/v1/sessions")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({}))
        .await;
}
```

---

## Test Dependencies

### Primary Testing Libraries

```toml
[dev-dependencies]
reqwest = { version = "0.12", features = ["json"] }  # HTTP client
axum-test = "18"          # Axum integration testing
tokio-test = "0.4"        # Async test utilities
mockito = "1.7"           # HTTP mocking (for future use)
tempfile = "3"            # Temporary files
```

**Why axum-test?**
- Provides `TestServer` for in-process HTTP testing
- No need to bind to actual ports
- Clean request/response API
- Header and status code assertions

**Why unique database per test?**
- SQLite `:memory:` databases are connection-specific
- Using `file:test_{uuid}?mode=memory&cache=shared` ensures isolation
- Prevents race conditions when tests run in parallel
- Each test gets clean state

---

## Writing New Tests

### Unit Test Template

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_my_function() {
        // Arrange
        let input = "test data";

        // Act
        let result = my_function(input);

        // Assert
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), expected_value);
    }
}
```

### Integration Test Template

```rust
mod common;

use axum_test::TestServer;
use common::TestApp;
use serde_json::json;

#[tokio::test]
async fn test_my_endpoint() {
    // Setup
    let test_app = TestApp::new().await;
    let app = test_app.build_app();
    let server = TestServer::new(app).unwrap();

    // Make request
    let response = server
        .post("/api/v1/endpoint")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .json(&json!({"key": "value"}))
        .await;

    // Assert
    response.assert_status_ok();
    let body: serde_json::Value = response.json();
    assert_eq!(body["field"], "expected");
}
```

---

## Test Data Validation

### JPEG Image Validation

Tests verify JPEG files by checking the signature:

```rust
let jpeg_signature: [u8; 3] = [255, 216, 255]; // 0xFF 0xD8 0xFF
assert!(bytes.starts_with(&jpeg_signature), "Should be valid JPEG");
assert!(bytes.len() > 1000, "Should have substantial data");
```

### Base64 Validation

For JSON endpoint tests:

```rust
use base64::Engine;

let decoded = base64::Engine::decode(
    &base64::engine::general_purpose::STANDARD,
    base64_data,
);
assert!(decoded.is_ok(), "Should be valid base64");
```

### HTTP Header Validation

For binary endpoint:

```rust
let headers = response.headers();
assert_eq!(headers.get("content-type").unwrap(), "image/jpeg");
assert!(headers.get("etag").is_some());
assert!(headers.get("cache-control").is_some());
assert!(headers.get("expires").is_some());
```

---

## Continuous Integration

### Pre-commit Checks

**REQUIRED** before every commit:

```bash
# 1. Format code
cargo fmt

# 2. Run linter
cargo clippy --all-targets

# 3. Run all tests
cargo test
```

### CI Pipeline Recommendation

```yaml
# .github/workflows/ci.yml
name: CI

on: [push, pull_request]

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable

      - name: Format check
        run: cargo fmt -- --check

      - name: Clippy
        run: cargo clippy --all-targets -- -D warnings

      - name: Run tests
        run: cargo test --all-targets

      - name: Build release
        run: cargo build --release
```

---

## Performance Testing

### Load Testing

Use a tool like `wrk` or `hey` to test performance:

```bash
# Install hey
go install github.com/rakyll/hey@latest

# Test session creation
hey -n 1000 -c 10 \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -m POST \
  -d '{"difficulty": 5}' \
  http://localhost:3000/api/v1/sessions

# Test image retrieval
hey -n 1000 -c 10 \
  http://localhost:3000/api/v1/sessions/{session_id}/image.jpeg
```

### Benchmarking

```bash
# Run benchmarks (if implemented)
cargo bench

# Profile with flamegraph
cargo flamegraph --test sessions_test
```

---

## Common Test Patterns

### Testing Authentication Failures

```rust
let response = server
    .post("/api/v1/sessions")
    // No Authorization header
    .json(&json!({}))
    .await;

response.assert_status_unauthorized();
```

### Testing Validation

```rust
let response = server
    .post("/api/v1/sessions")
    .add_header("Authorization", format!("Bearer {}", test_app.api_key))
    .json(&json!({"difficulty": 15}))  // Invalid: out of range
    .await;

response.assert_status_bad_request();
```

### Testing Complete Flows

```rust
// 1. Create
let create_resp = server.post("/api/v1/sessions")...await;
let session_id = create_resp.json()["session_id"].as_str().unwrap();

// 2. Use
let image_resp = server.get(&format!("/{}", session_id))...await;

// 3. Validate
let valid_resp = server.post(&format!("/{}/validate", session_id))...await;

// 4. Verify cleanup
let should_be_404 = server.get(&format!("/{}", session_id))...await;
should_be_404.assert_status_not_found();
```

---

## Debugging Failed Tests

### Enable Detailed Output

```bash
# Show all println!/eprintln! output
cargo test -- --nocapture

# Show test names as they run
cargo test -- --nocapture --test-threads=1

# Run with backtrace
RUST_BACKTRACE=1 cargo test
RUST_BACKTRACE=full cargo test
```

### Common Issues

#### "Database schema is locked"
- **Cause**: Tests running in parallel share the same in-memory database
- **Solution**: Already fixed - each test gets unique database via UUID

#### "no such table: api_keys"
- **Cause**: Migration didn't run or database not shared across connections
- **Solution**: Using `file:test_{uuid}?mode=memory&cache=shared`

#### "Expected 201, got 200"
- **Cause**: Endpoint not returning proper HTTP status code
- **Solution**: Return `(StatusCode::CREATED, Json(...))` tuple

---

## Test Isolation

### Database Isolation

Each test creates its own database:

```rust
let db_name = format!("file:test_{}?mode=memory&cache=shared", Uuid::new_v4());
```

**Why this approach?**
- Tests can run in parallel without conflicts
- Clean state for each test
- Fast (in-memory)
- Shared across connections within the same test

### No Test Data Leakage

- Each `TestApp::new()` call creates fresh database
- Migrations run automatically
- Test API key created per instance
- No cleanup needed (in-memory databases are automatically freed)

---

## Test Assertions

### Status Code Assertions

```rust
response.assert_status_ok();                          // 200
response.assert_status(StatusCode::CREATED);          // 201
response.assert_status_unauthorized();                // 401
response.assert_status_not_found();                   // 404
response.assert_status_bad_request();                 // 400
```

### JSON Assertions

```rust
let body: serde_json::Value = response.json();

assert!(body.get("session_id").is_some());
assert_eq!(body["valid"], true);
assert_eq!(body["description"], "Test Key");
```

### Binary Data Assertions

```rust
let bytes = response.as_bytes();

// JPEG signature
let jpeg_sig: [u8; 3] = [255, 216, 255];
assert!(bytes.starts_with(&jpeg_sig));

// Size check
assert!(bytes.len() > 1000);
```

### Header Assertions

```rust
let headers = response.headers();

assert_eq!(headers.get("content-type").unwrap(), "image/jpeg");
assert!(headers.get("etag").is_some());
assert!(headers.get("cache-control").is_some());
```

---

## Mock Data

### Test Constants

```rust
// From tests/common/mod.rs
const TEST_SALT: &str = "test-salt";
const TEST_API_KEY: &str = "test-api-key-123";
const TEST_MASTER_KEY: &str = "test-master-key";
```

### Generated Test Data

```rust
// API keys: 32-character alphanumeric
"HmLHQ6ou3kchYrMnQ9UPau6mLi1KXCBO"

// Session IDs: UUID v4
"550e8400-e29b-41d4-a716-446655440000"

// CAPTCHA text: 5-character alphanumeric (default)
"Ab3X9"
```

---

## Test Coverage Goals

### Current Coverage

- ✅ Happy path scenarios
- ✅ Authentication and authorization
- ✅ Validation and error handling
- ✅ Database operations
- ✅ Image format validation
- ✅ HTTP headers and caching
- ✅ Complete user flows

### Areas for Future Coverage

- ⚠️ Concurrent request handling
- ⚠️ Session expiration (time-based)
- ⚠️ Cleanup task execution
- ⚠️ Rate limiting (when implemented)
- ⚠️ CORS configuration (when implemented)
- ⚠️ Maximum session limits
- ⚠️ Database connection pool exhaustion

---

## Performance Benchmarks

### Test Execution Time

Typical test run times:
- Unit tests: ~0.15s (22 tests)
- Integration tests: ~0.60s (17 tests)
- Total: ~1.5s (40 tests)

### Database Operations

In-memory SQLite operations are very fast:
- Session create: < 1ms
- Session retrieve: < 1ms
- Validation: < 2ms (includes delete)

---

## Extending Tests

### Adding a New Unit Test

1. Add test function in relevant service file
2. Use `#[test]` attribute
3. Follow AAA pattern (Arrange, Act, Assert)
4. Run `cargo test --lib`

### Adding a New Integration Test

1. Add test to appropriate file in `tests/`
2. Use `#[tokio::test]` attribute
3. Use `TestApp::new()` for setup
4. Make HTTP requests via `TestServer`
5. Assert responses

### Testing New Endpoints

```rust
#[tokio::test]
async fn test_new_endpoint() {
    let test_app = TestApp::new().await;
    let server = TestServer::new(test_app.build_app()).unwrap();

    let response = server
        .get("/new/endpoint")
        .add_header("Authorization", format!("Bearer {}", test_app.api_key))
        .await;

    response.assert_status_ok();
    // Add specific assertions
}
```

---

## Test Best Practices

### DO:
- ✅ Test both success and failure cases
- ✅ Validate response structure, not just status codes
- ✅ Test authentication on all protected endpoints
- ✅ Verify data is actually saved to database
- ✅ Check binary data signatures (JPEG, PNG, etc.)
- ✅ Test complete user flows, not just isolated operations
- ✅ Use descriptive test names: `test_<action>_<condition>_<expected_result>`

### DON'T:
- ❌ Share state between tests
- ❌ Rely on test execution order
- ❌ Use sleep() for timing (use actual conditions)
- ❌ Hardcode timestamps (use relative times)
- ❌ Skip error case testing
- ❌ Test implementation details (test behavior)

---

## Troubleshooting Tests

### Tests Fail Locally but Pass in CI
- Check for timing issues
- Verify no hardcoded paths
- Ensure no environment dependencies

### Tests Pass Individually but Fail Together
- Database isolation issue
- Shared state problem
- Check TestApp creates unique database

### Slow Tests
- Most tests should run in < 100ms
- If slow, check for:
  - Actual network requests (should use mocks)
  - Large image generation
  - Inefficient database queries

---

## Code Quality Standards

**After EVERY code change, you MUST run these steps IN ORDER:**

### Step 1: Format Code
```bash
cargo fmt
```
Ensures consistent code formatting across the project.

### Step 2: Run Linter
```bash
cargo clippy --all-targets
```
Catches common mistakes and anti-patterns.

### Step 3: Verify Compilation
```bash
cargo check
```
Ensures code compiles successfully.

### Step 4: Run Rust Tests
```bash
cargo test
```
Runs all unit tests (22) and integration tests (17).

### Step 5: Run API Tests
**IMPORTANT:** API tests require a running server instance.

```bash
# Terminal 1: Start the server
cargo run

# Terminal 2: Run API tests
./.bruno/Tests/Scripts/test-bruno-full.sh
```

Runs comprehensive API test suite (18 requests, 38 tests).

**All five steps must pass with no errors before committing.**

### Why Each Step Matters

- **cargo fmt** - Maintains code consistency
- **cargo clippy** - Prevents common bugs
- **cargo check** - Catches compilation errors
- **cargo test** - Validates internal logic and component interactions
- **API tests** - Validates the actual HTTP interface clients will use

**CRITICAL:** Changes may pass Rust tests but break the HTTP API. Always run both test suites to ensure:
- ✅ Internal logic is correct (Rust tests)
- ✅ HTTP interface works as expected (API tests)
- ✅ Error responses are properly formatted
- ✅ Authentication flows work end-to-end
- ✅ HTTP headers and status codes are correct

---

## Writing and Updating Tests

### CRITICAL: Test-Driven Development

**After EVERY code change, you MUST update BOTH test suites:**

#### When Adding New Features

1. **Write Rust tests first:**
   ```bash
   # Add unit tests in src/services/your_service.rs
   # Add integration tests in tests/your_feature_test.rs
   cargo test
   ```

2. **Write API tests:**
   ```bash
   # Add new .bru files in .bruno/Tests/
   # Update test scripts if needed
   # Run: ./.bruno/Tests/Scripts/test-bruno-full.sh
   ```

3. **Update core endpoints:**
   ```bash
   # Add corresponding .bru files in .bruno/ for normal usage
   ```

#### When Modifying Existing Features

1. **Update Rust tests:**
   - Modify existing unit tests to match new behavior
   - Update integration tests
   - Add new test cases for edge cases
   - Run: `cargo test`

2. **Update API tests:**
   - Update .bru files in `.bruno/Tests/`
   - Update expected responses
   - Add new failure scenarios
   - Run: `./.bruno/Tests/Scripts/test-bruno-full.sh` (with server running)

3. **Update core endpoints:**
   - Update .bru files in `.bruno/` to match new API

#### When Fixing Bugs

1. **Write regression test first (Rust):**
   ```rust
   #[test]
   fn test_bug_xyz_fixed() {
       // Test that reproduces the bug
       // This should FAIL initially
   }
   ```

2. **Write API regression test:**
   ```
   # Add .bru file that reproduces the bug via HTTP
   ```

3. **Fix the bug**

4. **Verify both tests now pass:**
   ```bash
   cargo test
   # Then run API tests with server
   ```

### Test Coverage Requirements

**For every new endpoint, you MUST have:**

✅ **Rust Integration Tests:**
- Success case (happy path)
- Authentication failure (401)
- Invalid parameters (400)
- Not found scenario (404) if applicable

✅ **Bruno API Tests (in .bruno/Tests/):**
- Success case with actual HTTP request
- Unauthorized access test
- Invalid parameters test
- Not found test (if applicable)

✅ **Bruno Core Endpoint (in .bruno/):**
- Clean endpoint for normal usage
- Manual placeholders
- Proper documentation

**Example:** Adding a new `GET /api/v1/sessions/:id/stats` endpoint:

```bash
# 1. Write Rust tests
tests/sessions_test.rs:
  - test_get_session_stats_success()
  - test_get_session_stats_not_found()
  - test_get_session_stats_unauthorized()

# 2. Write Bruno test scenarios
.bruno/Tests/Sessions/:
  - Get Session Stats.bru (success)
  - Get Session Stats - Not Found.bru
  - Get Session Stats - Unauthorized.bru

# 3. Add Bruno core endpoint
.bruno/Sessions/:
  - Get Session Stats.bru (for normal usage)

# 4. Update test script
.bruno/Tests/Scripts/test-bruno-full.sh:
  Add new test files to execution list
```

---

## Test Maintenance

### When to Update Tests

- ✅ After adding new endpoints (WRITE TESTS FIRST!)
- ✅ After changing request/response formats
- ✅ After modifying database schema
- ✅ After changing business logic
- ✅ When fixing bugs (add regression test FIRST!)
- ✅ After security fixes (add security test)

### Keeping Tests Fast

- Use in-memory databases (already implemented)
- Avoid unnecessary setup in each test
- Share `TestApp` creation pattern
- Don't test external dependencies (use mocks)

---

## API Tests (Bruno Collection)

In addition to Rust unit and integration tests, the project includes comprehensive API tests using Bruno.

### Overview

**Location**: `.bruno/` directory

**Total API Tests**: 38 tests across 18 requests
- Core endpoints: 10 requests (normal API usage)
- Test scenarios: 11 requests (automated testing)

### Running API Tests

**Prerequisites:** Server must be running before running API tests.

```bash
# Terminal 1: Start the server
cargo run

# Terminal 2: Run API tests
# Quick happy path test (6 requests, 16 tests)
./.bruno/Tests/Scripts/test-bruno.sh

# OR comprehensive test suite (18 requests, 38 tests)
./.bruno/Tests/Scripts/test-bruno-full.sh
```

**Expected output:**
```
📊 Execution Summary
┌───────────────┬────────────────┐
│ Status        │     ✓ PASS     │
│ Requests      │ 18 (18 Passed) │
│ Tests         │     38/38      │
│ Duration (ms) │      ~170      │
└───────────────┴────────────────┘
```

**Note:** If API tests fail, check that:
- Server is running on http://127.0.0.1:3000
- `.env` file has correct `MASTER_API_KEY`
- Database is accessible (created automatically on startup)

### What API Tests Cover

**Success Scenarios:**
- ✅ Health check endpoint
- ✅ API key management (create, list, update, delete)
- ✅ Session lifecycle (create, retrieve images, validate, delete)
- ✅ Image formats (JSON base64, binary JPEG)
- ✅ HTTP headers and caching

**Failure Scenarios:**
- ❌ Unauthorized access (missing/invalid auth)
- ❌ Invalid parameters (e.g., difficulty=99)
- ❌ Not found errors (non-existent resources)
- ❌ Business logic failures (max attempts, wrong solutions)

### Bruno Test Organization

**Core Collection** (`.bruno/` root):
- For interactive GUI testing
- Manual placeholders
- Clean, simple endpoints

**Test Collection** (`.bruno/Tests/`):
- For automated CI/CD testing
- Auto-populated variables
- Success + failure scenarios
- Test scripts and documentation

### API Test Documentation

- **`.bruno/README.md`** - Main collection overview
- **`.bruno/ORGANIZATION.md`** - Organization guide
- **`.bruno/Tests/README.md`** - Test-specific docs
- **`.bruno/Tests/Documentation/TEST-SCENARIOS.md`** - All scenarios

### Integration with Development

API tests validate the entire HTTP layer:
- Request/response formats
- Authentication flows
- Error messages
- HTTP status codes
- Header correctness

These complement Rust tests by testing the **actual HTTP interface** that clients use.

---

## Continuous Integration

### GitHub Actions Workflow

The project includes automated testing via GitHub Actions (`.github/workflows/ci.yml`).

#### What Gets Tested

On every push and pull request, jobs run in **3 phases**:

**Phase 1: Quality Gates (Parallel, ~1-2 min)**
   - `format` - Code formatting check (~30s)
   - `clippy` - Linting (~1-2min)

   **Gates**: Tests don't run if format or clippy fail

**Phase 2: Core Tests (needs: format + clippy, ~2-3 min)**
   - `test` - Unit and integration tests (40 tests)

**Phase 3: Advanced Tests (Parallel, needs: test, ~3-4 min)**
   - `coverage` - Code coverage report (85% threshold)
   - `api-tests` - HTTP API validation (38 tests)
     - Builds project
     - Starts server in background
     - Waits for server readiness
     - Runs Bruno CLI tests
     - Cleans up server

**Why this structure?**
- **Fail fast**: Format/clippy catch 30% of issues in ~1-2min
- **Gate strategy**: Don't run tests on malformed code
- **Parallel where safe**: Phase 1 and Phase 3 run in parallel
- **Sequential where needed**: Tests only after gates pass
- **Saves 3-4min on 80% of failures**

#### Environment Configuration

CI uses a dedicated environment (`.bruno/environments/ci.bru`) with test credentials:
- Master key: `ci-test-master-key-do-not-use-in-production`
- Auto-generated API keys during test execution

#### Viewing CI Results

1. Go to **Actions** tab in GitHub
2. Click on workflow run
3. View job results and logs
4. Download artifacts for failed tests

#### Running Tests with CI Environment Locally

```bash
# Start server
cargo run

# Run with CI environment
BRUNO_ENV=ci ./.bruno/Tests/Scripts/test-bruno-full.sh
```

See `.github/workflows/README.md` for complete CI/CD documentation.

---

## Future Test Improvements

### Potential Additions

1. **Property-based testing** with `proptest`
   - Random input generation
   - Edge case discovery

2. **Mutation testing** with `cargo-mutants`
   - Verify tests actually catch bugs

3. **Coverage reporting** with `tarpaulin`
   - Measure code coverage percentage

4. **Performance regression tests**
   - Benchmark critical paths
   - Alert on slowdowns

5. **Chaos testing**
   - Simulate database failures
   - Network interruptions
   - Resource exhaustion

---

**Last Updated**: 2025-10-23
**Test Count**: 40
**Coverage**: Unit + Integration + Migration
