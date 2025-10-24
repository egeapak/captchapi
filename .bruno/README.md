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
- **Sessions > Get Image (JSON)**: Returns base64-encoded image in JSON
- **Sessions > Get Image (Binary)**: Returns raw JPEG image

Open the image to see the CAPTCHA challenge.

### 4. Validate the Solution
Run: **Sessions > Validate Session**
- Update the `solution` field in the request body with your answer
- Case-insensitive matching
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
├── .gitignore                          # Git ignore rules
├── environments/                       # Environment configurations
│   ├── local.bru                      # Local development
│   └── production.bru                 # Production template
├── Health Check.bru                   # Health endpoint
├── API Keys/                          # Core API key endpoints
│   ├── Create API Key.bru            # Generate new key
│   ├── List API Keys.bru             # List all keys
│   ├── Update API Key.bru            # Modify key (manual hash)
│   └── Delete API Key.bru            # Remove key (manual hash)
├── Sessions/                          # Core session endpoints
│   ├── Create Session.bru            # Generate CAPTCHA
│   ├── Get Image (JSON).bru          # Base64 image
│   ├── Get Image (Binary).bru        # Raw JPEG
│   ├── Validate Session.bru          # Validate solution
│   └── Delete Session.bru            # Delete session
└── Tests/                             # Test-specific endpoints & utilities
    ├── README.md                      # Test documentation
    ├── API Keys/                      # API key test scenarios
    │   ├── Create API Key - Unauthorized.bru
    │   ├── Create API Key for Testing.bru
    │   ├── Update API Key (Test).bru
    │   ├── Delete API Key (Test).bru
    │   └── Delete API Key - Not Found.bru
    ├── Sessions/                      # Session test scenarios
    │   ├── Create Session - Unauthorized.bru
    │   ├── Create Session - Invalid Parameters.bru
    │   ├── Get Image - Not Found.bru
    │   ├── Validate Session - Wrong Answer.bru
    │   ├── Validate Session - Max Attempts.bru
    │   └── Validate Session - Session Deleted.bru
    ├── Scripts/                       # Test automation
    │   ├── test-bruno.sh             # Quick happy path test
    │   └── test-bruno-full.sh        # Comprehensive test suite
    └── Documentation/                 # Test documentation
        └── TEST-SCENARIOS.md         # Detailed scenarios

Core Endpoints: 10 requests (for normal API usage)
Test Endpoints: 11 requests (for automated testing)
Total: 21 requests
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

# Quick happy path test (6 requests, 16 tests)
./.bruno/Tests/Scripts/test-bruno.sh

# Comprehensive test suite (18 requests, 38 tests)
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
