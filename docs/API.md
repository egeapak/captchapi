# CaptchAPI - API Documentation

Complete API reference for the CaptchAPI service.

## Base URL

```
http://localhost:3000
```

## Authentication

### API Key Authentication
Protected endpoints require an API key passed via the `Authorization` header:

```
Authorization: Bearer <api_key>
```

### Master Key Authentication
Admin endpoints require the master key:

```
Authorization: Bearer <master_key>
```

---

## Endpoints Overview

### Public Endpoints
- `GET /health` - Health check
- `GET /api/v1/sessions/{id}` - Get session details (metadata only)
- `GET /api/v1/sessions/{id}/image.jpeg` - Retrieve CAPTCHA as binary JPEG (for browser display)

### Protected Endpoints (Require API Key)
- `POST /api/v1/sessions` - Create new CAPTCHA session
- `POST /api/v1/sessions/{id}/validate` - Validate user solution
- `DELETE /api/v1/sessions/{id}` - Delete session

### Admin Endpoints (Require Master Key)
- `POST /api/v1/api-keys` - Create new API key
- `GET /api/v1/api-keys` - List all API keys
- `PUT /api/v1/api-keys/{key_hash}` - Update API key (activate/deactivate)
- `DELETE /api/v1/api-keys/{key_hash}` - Delete API key

---

## Public Endpoints

### Health Check

Check service health status.

```http
GET /health
```

**Response: 200 OK**
```json
{
  "status": "healthy"
}
```

---

## Session Endpoints

### Create Session

Create a new CAPTCHA session with optional custom parameters.

```http
POST /api/v1/sessions
Authorization: Bearer <api_key>
Content-Type: application/json
```

**Request Body:**
```json
{
  "length": 5,                  // Optional: CAPTCHA text length 1-20 (default: 5)
  "expires_in_seconds": 300,    // Optional: TTL in seconds (default: 300)
  "difficulty": 5,              // Optional: 1-10 (default: 5)
  "width": 220,                 // Optional: pixels (default: 220)
  "height": 120,                // Optional: pixels (default: 120)
  "dark_mode": false,           // Optional: theme (default: false)
  "compression": 40             // Optional: JPEG quality 1-100 (default: 40)
}
```

**Response: 201 Created**
```json
{
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "expires_at": "2025-10-23T12:35:00Z",
  "created_at": "2025-10-23T12:30:00Z"
}
```

**Example:**
```bash
curl -X POST http://localhost:3000/api/v1/sessions \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "length": 6,
    "difficulty": 5,
    "expires_in_seconds": 300,
    "width": 220,
    "height": 120,
    "dark_mode": false
  }'
```

**Validation:**
- `length` must be between 1 and 20 characters
- `expires_in_seconds` cannot exceed `MAX_SESSION_TTL_SECONDS` (default: 3600)
- `difficulty` must be between 1 and 10
- `width` must be between 50 and 1000 pixels
- `height` must be between 30 and 500 pixels
- `compression` must be between 1 and 100 (JPEG quality)
- All parameters are optional

---

### Get Session Details

Retrieve metadata about an existing CAPTCHA session (no image data).

```http
GET /api/v1/sessions/{id}
```

**Response: 200 OK**
```json
{
  "session_id": "550e8400-e29b-41d4-a716-446655440000",
  "created_at": "2025-01-15T10:35:00Z",
  "expires_at": "2025-01-15T10:40:00Z",
  "attempt_count": 0,
  "difficulty": 5,
  "width": 220,
  "height": 120,
  "dark_mode": false
}
```

**Example:**
```bash
curl http://localhost:3000/api/v1/sessions/550e8400-e29b-41d4-a716-446655440000
```

**Error Responses:**
- `404 Not Found` - Session does not exist or has expired

---

### Get CAPTCHA Image (Binary JPEG)

Retrieve the CAPTCHA image as raw JPEG bytes for direct browser display.

```http
GET /api/v1/sessions/{id}/image.jpeg
```

**Response: 200 OK**
- **Content-Type**: `image/jpeg`
- **Body**: Raw JPEG binary data

**Response Headers:**
```
Content-Type: image/jpeg
ETag: "550e8400-e29b-41d4-a716-446655440000"
Cache-Control: public, max-age=300
Expires: Thu, 23 Oct 2025 12:35:00 GMT
```

**Example:**
```bash
# Download the image
curl http://localhost:3000/api/v1/sessions/550e8400-e29b-41d4-a716-446655440000/image.jpeg \
  --output captcha.jpeg

# Open directly in browser
http://localhost:3000/api/v1/sessions/550e8400-e29b-41d4-a716-446655440000/image.jpeg

# Use in HTML
<img src="http://localhost:3000/api/v1/sessions/{session_id}/image.jpeg" alt="CAPTCHA">
```

**Caching:**
- `max-age` is calculated based on time remaining until session expiration
- `ETag` is the session ID (can be used for conditional requests)
- `Expires` header provides absolute expiration time

**Error Responses:**
- `404 Not Found` - Session does not exist or has expired

---

### Validate Solution

Validate a user's solution to the CAPTCHA challenge.

```http
POST /api/v1/sessions/{id}/validate
Authorization: Bearer <api_key>
Content-Type: application/json
```

**Request Body:**
```json
{
  "solution": "ABCD5"
}
```

**Response: 200 OK**
```json
{
  "valid": true,
  "session_id": "550e8400-e29b-41d4-a716-446655440000"
}
```

**Example:**
```bash
curl -X POST http://localhost:3000/api/v1/sessions/550e8400-e29b-41d4-a716-446655440000/validate \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"solution": "ABCD5"}'
```

**Behavior:**
- Solutions are compared **case-sensitively** (exact match required)
- The answer is stored as a keyed hash, never in plaintext; comparison is constant-time
- The CAPTCHA image is stored encrypted and decrypted only when served
- Session is **automatically deleted** after successful validation
- Failed attempts are tracked
- Session is **deleted after 3 failed attempts**
- Expired sessions return `404 Not Found`

**Important:** Validation is case-sensitive. "aBc5X" and "abc5x" are considered different solutions.

**Error Responses:**
- `401 Unauthorized` - Invalid or missing API key
- `404 Not Found` - Session does not exist or has expired

---

### Delete Session

Prematurely delete a CAPTCHA session.

```http
DELETE /api/v1/sessions/{id}
Authorization: Bearer <api_key>
```

**Response: 204 No Content**

**Example:**
```bash
curl -X DELETE http://localhost:3000/api/v1/sessions/550e8400-e29b-41d4-a716-446655440000 \
  -H "Authorization: Bearer YOUR_API_KEY"
```

**Error Responses:**
- `401 Unauthorized` - Invalid or missing API key
- `404 Not Found` - Session does not exist

---

## API Key Management Endpoints

### Create API Key

Create a new API key for accessing protected endpoints.

```http
POST /api/v1/api-keys
Authorization: Bearer <master_key>
Content-Type: application/json
```

**Request Body:**
```json
{
  "description": "Production API Key"  // Optional: human-readable description
}
```

**Response: 201 Created**
```json
{
  "api_key": "HmLHQ6ou3kchYrMnQ9UPau6mLi1KXCBO",
  "key_hash": "fcc484955c95e3ed5d8a0f9991aae7c1f5e958e7cfa1d7bd3d762f7172871e64",
  "description": "Production API Key",
  "created_at": "2025-10-23T12:32:38Z"
}
```

**IMPORTANT**: Save the `api_key` value immediately - it will **never be shown again**!

**Example:**
```bash
curl -X POST http://localhost:3000/api/v1/api-keys \
  -H "Authorization: Bearer YOUR_MASTER_KEY" \
  -H "Content-Type: application/json" \
  -d '{"description": "Production API Key"}'
```

**Notes:**
- The API key is a randomly generated 32-character alphanumeric string
- Only the hashed version is stored in the database
- The plaintext key is only returned once in this response

---

### List API Keys

Retrieve all API keys (without the actual key values).

```http
GET /api/v1/api-keys
Authorization: Bearer <master_key>
```

**Response: 200 OK**
```json
[
  {
    "key_hash": "fcc484955c95e3ed5d8a0f9991aae7c1f5e958e7cfa1d7bd3d762f7172871e64",
    "description": "Production API Key",
    "created_at": "2025-10-23T12:32:38Z",
    "last_used_at": "2025-10-23T13:00:00Z",
    "is_active": true
  },
  {
    "key_hash": "a1b2c3...",
    "description": "Development Key",
    "created_at": "2025-10-23T10:00:00Z",
    "last_used_at": null,
    "is_active": false
  }
]
```

**Example:**
```bash
curl http://localhost:3000/api/v1/api-keys \
  -H "Authorization: Bearer YOUR_MASTER_KEY"
```

---

### Update API Key

Update an API key's status or description.

```http
PUT /api/v1/api-keys/{key_hash}
Authorization: Bearer <master_key>
Content-Type: application/json
```

**Request Body:**
```json
{
  "is_active": false,              // Optional: activate/deactivate key
  "description": "New description" // Optional: update description
}
```

**Response: 200 OK**
```json
{
  "key_hash": "fcc484955c95...",
  "description": "New description",
  "created_at": "2025-10-23T12:32:38Z",
  "last_used_at": "2025-10-23T13:00:00Z",
  "is_active": false
}
```

**Example - Deactivate a key:**
```bash
curl -X PUT http://localhost:3000/api/v1/api-keys/fcc484955c95e3ed5d8a0f9991aae7c1f5e958e7cfa1d7bd3d762f7172871e64 \
  -H "Authorization: Bearer YOUR_MASTER_KEY" \
  -H "Content-Type: application/json" \
  -d '{"is_active": false}'
```

**Notes:**
- Deactivated keys cannot be used to access protected endpoints
- At least one field (`is_active` or `description`) must be provided
- Returns `404 Not Found` if key doesn't exist

---

### Delete API Key

Permanently delete an API key.

```http
DELETE /api/v1/api-keys/{key_hash}
Authorization: Bearer <master_key>
```

**Response: 204 No Content**

**Example:**
```bash
curl -X DELETE http://localhost:3000/api/v1/api-keys/fcc484955c95e3ed5d8a0f9991aae7c1f5e958e7cfa1d7bd3d762f7172871e64 \
  -H "Authorization: Bearer YOUR_MASTER_KEY"
```

**Error Responses:**
- `401 Unauthorized` - Invalid or missing master key
- `404 Not Found` - API key does not exist

---

## Error Responses

All errors return JSON responses with this format:

```json
{
  "error": "error_code",
  "message": "Human-readable description"
}
```

### Common Error Codes

| Code | HTTP Status | Description |
|------|-------------|-------------|
| `session_not_found` | 404 | Session doesn't exist or expired |
| `unauthorized` | 401 | Invalid or missing API/master key |
| `invalid_parameters` | 400 | Bad request parameters |
| `database_error` | 500 | Internal database error |
| `internal_error` | 500 | Other internal errors |

---

## Complete Usage Flow

### 1. Setup (One-time)

```bash
# Create an API key using master key
curl -X POST http://localhost:3000/api/v1/api-keys \
  -H "Authorization: Bearer YOUR_MASTER_KEY" \
  -H "Content-Type: application/json" \
  -d '{"description": "Web App Key"}'

# Save the returned api_key value!
```

### 2. Create CAPTCHA Session

```bash
# Create a session
SESSION_RESPONSE=$(curl -X POST http://localhost:3000/api/v1/sessions \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"length": 5, "difficulty": 5}')

SESSION_ID=$(echo $SESSION_RESPONSE | jq -r '.session_id')
```

### 3. Display CAPTCHA to User

```html
<img src="http://localhost:3000/api/v1/sessions/{session_id}/image.jpeg"
     alt="CAPTCHA">
```

### 4. Validate User Input

```bash
# Submit the user's typed solution
curl -X POST http://localhost:3000/api/v1/sessions/$SESSION_ID/validate \
  -H "Authorization: Bearer YOUR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"solution": "USER_TYPED_ANSWER"}'

# Response: {"valid": true/false, "session_id": "..."}
```

**Note:** Validation is case-sensitive. The solution text is not returned by the API — the user must read and type it from the displayed CAPTCHA image.

---

## Rate Limiting & Best Practices

### Current Implementation
- Rate limiting is implemented using tower_governor with GCRA algorithm. Configure with `RATE_LIMIT_REQUESTS_PER_SECOND` (default: 2) and `RATE_LIMIT_BURST_SIZE` (default: 10). Set `RATE_LIMIT_REVERSE_PROXY=true` when behind a reverse proxy.
- Session cleanup runs every 60 seconds (configurable)
- Max 3 validation attempts per session

### Recommended Practices

1. **Session Management**
   - Set appropriate TTL based on your use case (default: 300s)
   - Don't reuse session IDs
   - Delete sessions immediately after successful validation

2. **API Key Security**
   - Rotate keys periodically
   - Use different keys for different environments (dev/staging/prod)
   - Deactivate compromised keys immediately
   - Monitor `last_used_at` timestamps for suspicious activity

3. **CAPTCHA Configuration**
   - Difficulty 1-3: Easy (good UX, less secure)
   - Difficulty 4-6: Medium (balanced)
   - Difficulty 7-10: Hard (better security, worse UX)
   - Consider using dark mode for dark-themed sites

4. **Caching**
   - Binary endpoint includes proper cache headers
   - Browsers will cache images until expiration
   - Use ETag for conditional requests

---

## Response Headers (Binary Endpoint)

The `/api/v1/sessions/{id}/image.jpeg` endpoint includes:

```
Content-Type: image/jpeg
ETag: "{session_id}"
Cache-Control: public, max-age={seconds_until_expiration}
Expires: {RFC2822_datetime}
```

These headers optimize browser caching and reduce server load.

---

## Migration from Other CAPTCHA Services

### From Google reCAPTCHA

**Before:**
```javascript
// Client-side
grecaptcha.execute()

// Server-side verification
fetch('https://www.google.com/recaptcha/api/siteverify', ...)
```

**After (CaptchAPI):**
```javascript
// 1. Create session (server-side)
const session = await createCaptchaSession();

// 2. Display image (client-side)
<img src={`/api/v1/sessions/${session.session_id}/image.jpeg`} />

// 3. Validate (server-side)
const result = await validateCaptcha(sessionId, userInput);
```

### Benefits
- Self-hosted (no external dependencies)
- No JavaScript required on client
- Full control over difficulty and appearance
- No user tracking
- GDPR-friendly

---

## Troubleshooting

### "Session not found" errors
- Check that session hasn't expired (default: 5 minutes)
- Verify session ID is correct
- Ensure session wasn't already validated (sessions auto-delete on success)

### "Unauthorized" errors
- Verify API key is correct
- Check `Authorization: Bearer <key>` format
- Ensure key is active (check via `/api/v1/api-keys` endpoint)
- For admin endpoints, use master key not API key

### Images not displaying
- Verify Content-Type header is `image/jpeg`
- Check browser console for CORS errors
- Ensure session hasn't expired
- Try accessing binary endpoint directly in browser

---

## API Versioning

Current version: **v1**

The API is versioned via URL path (`/api/v1/...`). Breaking changes will increment the version number.

---

**Last Updated**: 2026-02-25
**API Version**: v1
**Service Version**: 1.0.0
