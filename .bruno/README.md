# CaptchAPI Bruno Collection

This Bruno collection provides ready-to-use API requests for testing the CaptchAPI service.

## Setup

1. **Install Bruno**: Download from https://www.usebruno.com/downloads

2. **Open Collection**: In Bruno, click "Open Collection" and select the `.bruno` folder

3. **Configure Environment**:
   - Select "local" environment from the dropdown in the top-right
   - The collection uses these environment variables:
     - `base_url`: http://127.0.0.1:3000
     - `master_key`: change-this-to-a-secure-master-key-in-production
     - `api_key`: Will be auto-populated after creating an API key
     - `session_id`: Will be auto-populated after creating a session

## Usage Flow

### 1. Create an API Key
Run: **API Keys > Create API Key**
- This automatically saves the returned `api_key` to your environment
- You'll need this key for all session operations

### 2. Create a CAPTCHA Session
Run: **Sessions > Create Session**
- This automatically saves the returned `session_id` to your environment
- Customize difficulty, dimensions, and dark mode in the request body

### 3. Get the CAPTCHA Image
Run either:
- **Sessions > Get Image (Binary)**: Returns raw JPEG image

Open the image to see the CAPTCHA challenge.

### 4. Validate the Solution
Run: **Sessions > Validate Session**
- Update the `solution` field in the request body with your answer
- Case-sensitive matching
- Max 3 attempts per session

### 5. (Optional) Delete Session
Run: **Sessions > Delete Session**
- Manually delete a session if needed
- Sessions auto-delete on successful validation or expiration

## Features

### Automatic Variable Population
- Creating an API key automatically updates `{{api_key}}`
- Creating a session automatically updates `{{session_id}}`
- No manual copying needed!

### Built-in Tests
Each request includes tests that validate:
- Correct HTTP status codes
- Expected response structure
- Required headers (for binary image)

### Environment Support
- Pre-configured `local` environment
- Easy to add staging/production environments
- All variables use `{{var}}` syntax

## Collection Structure

```
.bruno/
├── bruno.json                          # Collection metadata
├── README.md                           # This file
├── ORGANIZATION.md                     # Core-vs-Tests rationale
├── environments/
│   ├── ci.bru                          # Used by CI (BRUNO_ENV=ci)
│   └── local.bru                       # Local development
├── Health Check.bru
├── API Keys/
│   ├── Create API Key.bru
│   ├── Delete API Key.bru
│   ├── List API Keys.bru
│   └── Update API Key.bru
├── Sessions/
│   ├── Create Session.bru
│   ├── Delete Session.bru
│   ├── Get Image (Binary).bru
│   ├── Get Session Details.bru
│   └── Validate Session.bru
├── Admin/
│   ├── Cleanup Expired Sessions.bru
│   ├── Get Config.bru
│   ├── Patch Config.bru
│   └── Reload Config.bru
└── Tests/
    ├── API Keys/
    │   ├── Create API Key - Unauthorized.bru
    │   ├── Create API Key for Testing.bru
    │   ├── Delete API Key (Test).bru
    │   ├── Delete API Key - Not Found.bru
    │   ├── List API Keys - Unauthorized.bru
    │   ├── Update API Key (Test).bru
    │   ├── Update API Key - Not Found.bru
    │   └── Update API Key - Unauthorized.bru
    ├── Admin/
    │   ├── Cleanup - Invalid Master Key.bru
    │   ├── Cleanup - Success.bru
    │   └── Cleanup - Unauthorized.bru
    ├── Admin Config/
    │   ├── Get Config - Invalid Master Key.bru
    │   ├── Get Config - Success.bru
    │   ├── Get Config - Unauthorized.bru
    │   ├── Patch Config - Invalid Value.bru
    │   ├── Patch Config - Not Reloadable.bru
    │   ├── Patch Config - Success.bru
    │   ├── Patch Config - Unauthorized.bru
    │   ├── Reload Config - Success.bru
    │   └── Reload Config - Unauthorized.bru
    ├── Documentation/
    │   └── TEST-SCENARIOS.md
    ├── Scripts/
    │   ├── README.md
    │   ├── test-bruno-full.sh
    │   └── test-bruno.sh
    ├── Sessions/
    │   ├── Create Session - Invalid Height.bru
    │   ├── Create Session - Invalid Length.bru
    │   ├── Create Session - Invalid Parameters.bru
    │   ├── Create Session - Invalid Width.bru
    │   ├── Create Session - Unauthorized.bru
    │   ├── Delete Session - Not Found.bru
    │   ├── Delete Session - Unauthorized.bru
    │   ├── Get Image - Not Found.bru
    │   ├── Validate Session - Max Attempts.bru
    │   ├── Validate Session - Session Deleted.bru
    │   ├── Validate Session - Unauthorized.bru
    │   └── Validate Session - Wrong Answer.bru
    └── README.md

Core requests: 14   Test requests: 32   Total .bru files: 46
Full suite run: 41 requests, 95 assertions (some requests run twice by design)
```

## Using the Collection

### Interactive Usage (Bruno GUI)

1. **Open in Bruno**: File → Open Collection → Select `.bruno` folder
2. **Select environment**: Choose "local" from dropdown
3. **Run requests**: Click on any request and hit "Send"
4. **Core endpoints**: Use the main folders (Health Check, API Keys, Sessions)
5. **Manual testing**: Update placeholder values like `YOUR_KEY_HASH_HERE`

### Automated Testing (Bruno CLI)

The collection includes automated test scripts for CI/CD:

```bash
# Install bruno-cli globally
npm install -g @usebruno/cli

# Quick happy path test (5 requests, 13 tests)
./.bruno/Tests/Scripts/test-bruno.sh

# Comprehensive test suite (41 requests, 95 tests)
./.bruno/Tests/Scripts/test-bruno-full.sh
```

**Test Coverage**:
- ✅ All success paths
- ❌ Unauthorized access
- ❌ Invalid parameters
- ❌ Not found errors
- ❌ Business logic failures

See **Tests/README.md** for detailed test documentation.

## Tips

- **Run requests in sequence**: Health Check → Create API Key → Create Session → Get Image → Validate
- **Check test results**: The "Tests" tab shows pass/fail for each request
- **View responses**: Switch between "Body", "Headers", and "Tests" tabs
- **Environment switching**: Easily switch between local/staging/prod environments
- **CLI testing**: Use `test-bruno.sh` script for automated testing

## Authentication

- **Master Key**: Required for API key management (create, list, update, delete)
- **API Key**: Required for session operations (create, validate, delete)
- **Public Access**: Health check and image retrieval don't require authentication

All authenticated requests use: `Authorization: Bearer <token>`
