# Bruno Test Scenarios

This document describes all test scenarios covered in the Bruno collection.

## Test Statistics

- **Total Requests**: 18
- **Total Tests**: 38
- **Coverage**: All 10 API endpoints
- **Scenarios**: Success paths + Error conditions

## Test Breakdown by Category

### 1. Health Check (1 request, 2 tests)

| Request | Status | Tests |
|---------|--------|-------|
| Health Check | 200 OK | Status code, response body |

### 2. API Key Management (8 requests, 12 tests)

| Request | Status | Scenario | Tests |
|---------|--------|----------|-------|
| Create API Key - Unauthorized | 401 | Missing master key | Status, error message |
| Create API Key | 201 | Valid creation | Status, API key returned |
| List API Keys | 200 | List all keys | Status, array response |
| Create API Key for Testing | 201 | Create key for update/delete | Status, key hash saved |
| Update API Key | 200 | Update description/active status | Status code |
| Delete API Key | 204 | Delete existing key | Status code |
| Delete API Key - Not Found | 404 | Delete non-existent key | Status, error message |

**Coverage**:
- ✅ POST /api/v1/api-keys (success + unauthorized)
- ✅ GET /api/v1/api-keys (success)
- ✅ PUT /api/v1/api-keys/:hash (success)
- ✅ DELETE /api/v1/api-keys/:hash (success + not found)

### 3. Session Management (9 requests, 24 tests)

| Request | Status | Scenario | Tests |
|---------|--------|----------|-------|
| Create Session - Unauthorized | 401 | Missing API key | Status, error message |
| Create Session - Invalid Parameters | 400 | difficulty=99 (out of range) | Status, error mentions difficulty |
| Create Session | 201 | Valid creation | Status, session_id, expires_at |
| Get Image (JSON) | 200 | Retrieve base64 image | Status, image format, expires_at |
| Get Image (Binary) | 200 | Retrieve raw JPEG | Status, content-type, cache headers, ETag |
| Get Image - Not Found | 404 | Non-existent session | Status, error message |
| Validate Session - Wrong Answer | 200 | Incorrect solution (attempt 1) | Status, valid=false, session_id |
| Validate Session - Max Attempts | 200 | Incorrect solution (attempt 2) | Status, valid=false |
| Validate Session - Max Attempts | 200 | Incorrect solution (attempt 3) | Status, valid=false |
| Validate Session - Session Deleted | 200 | Session deleted after max attempts | Status, valid=false |

**Coverage**:
- ✅ POST /api/v1/sessions (success + unauthorized + invalid params)
- ✅ GET /api/v1/sessions/:id/image (success + not found)
- ✅ GET /api/v1/sessions/:id/image.jpeg (success)
- ✅ POST /api/v1/sessions/:id/validate (success + failures)
- ✅ DELETE /api/v1/sessions/:id (implicit - via max attempts)

## Test Flow

The comprehensive test suite runs in this order:

```
1. Health Check ✓
2. API Keys - Unauthorized ✗
3. API Keys - Create (main) ✓ → saves api_key
4. API Keys - List ✓
5. API Keys - Create (for testing) ✓ → saves test_key_hash
6. API Keys - Update ✓
7. API Keys - Delete ✓
8. API Keys - Delete (not found) ✗
9. Sessions - Unauthorized ✗
10. Sessions - Invalid Params ✗
11. Sessions - Create ✓ → saves session_id
12. Sessions - Get Image JSON ✓
13. Sessions - Get Image Binary ✓
14. Sessions - Get Image (not found) ✗
15. Sessions - Validate (wrong #1) ✗
16. Sessions - Validate (wrong #2) ✗
17. Sessions - Validate (wrong #3) ✗ → session auto-deleted
18. Sessions - Validate (deleted) ✗
```

✓ = Success expected
✗ = Failure expected

## Variable Management

The test suite automatically manages these environment variables:

- `api_key` - Set by "Create API Key" (step 3)
- `session_id` - Set by "Create Session" (step 11)
- `test_key_hash` - Set by "Create API Key for Testing" (step 5)

These variables are used by subsequent requests, making the test flow completely automated.

## Error Scenarios Tested

### Authentication Errors
- Missing Authorization header → 401 Unauthorized
- Invalid master key → 401 Unauthorized
- Invalid API key → 401 Unauthorized

### Validation Errors
- Invalid difficulty (99) → 400 Bad Request with specific error
- Wrong CAPTCHA solution → 200 OK with valid=false
- Exceeding max attempts → Session deleted, valid=false

### Not Found Errors
- Non-existent session ID → 404 Not Found
- Non-existent API key hash → 404 Not Found

## Business Logic Tested

### Session Lifecycle
1. ✅ Session creation with valid parameters
2. ✅ Session creation fails without authentication
3. ✅ Session creation fails with invalid params
4. ✅ Image retrieval works with valid session
5. ✅ Image retrieval fails with invalid session
6. ✅ Validation accepts attempts up to max (3)
7. ✅ Session auto-deletes after max failed attempts
8. ✅ Session becomes inaccessible after deletion

### API Key Lifecycle
1. ✅ Key creation requires master key
2. ✅ Keys can be created with descriptions
3. ✅ Keys can be listed
4. ✅ Keys can be updated (description, active status)
5. ✅ Keys can be deleted
6. ✅ Deleting non-existent key returns 404

## HTTP Headers Verified

- `Content-Type: application/json` (all JSON responses)
- `Content-Type: image/jpeg` (binary image)
- `Cache-Control: public, max-age=X` (image caching)
- `ETag: "session-id"` (image versioning)
- `Expires: <datetime>` (cache expiration)

## Running Specific Test Scenarios

```bash
cd .bruno

# Test only authentication failures
bru run "API Keys/Create API Key - Unauthorized.bru" \
        "Sessions/Create Session - Unauthorized.bru" --env local

# Test only validation flow
bru run "Sessions/Create Session.bru" \
        "Sessions/Validate Session - Wrong Answer.bru" \
        "Sessions/Validate Session - Max Attempts.bru" \
        "Sessions/Validate Session - Max Attempts.bru" \
        "Sessions/Validate Session - Session Deleted.bru" --env local

# Test only CRUD operations
bru run "API Keys/Create API Key for Testing.bru" \
        "API Keys/Update API Key.bru" \
        "API Keys/Delete API Key.bru" --env local
```

## Expected Results

All 18 requests should complete with their expected status codes, and all 38 tests should pass:

```
📊 Execution Summary
┌───────────────┬────────────────┐
│ Status        │     ✓ PASS     │
│ Requests      │ 18 (18 Passed) │
│ Tests         │     38/38      │
│ Duration (ms) │      ~150      │
└───────────────┴────────────────┘
```
