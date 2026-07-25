# Bruno Collection Organization

This document explains the organization and purpose of each part of the Bruno collection.

## Directory Structure

> **Folder tree:** see [README.md](README.md#collection-structure), which is kept in sync with
> the filesystem. Duplicating it here is what let three copies drift apart.

## Core Collection (14 requests)

**Purpose**: Normal API usage, interactive testing in Bruno GUI

**Location**: Root of `.bruno/` folder

**Characteristics**:
- Clean, straightforward endpoints
- Manual placeholder values (e.g., `YOUR_KEY_HASH_HERE`)
- No auto-population of variables
- Intended for human interaction
- Self-documenting request names

**Endpoints**:
1. `Health Check.bru` - Server health status
2. `API Keys/Create API Key.bru` - Generate new API key
3. `API Keys/List API Keys.bru` - List all API keys
4. `API Keys/Update API Key.bru` - Update key (manual hash)
5. `API Keys/Delete API Key.bru` - Delete key (manual hash)
6. `Sessions/Create Session.bru` - Generate CAPTCHA
8. `Sessions/Get Image (Binary).bru` - Raw JPEG
9. `Sessions/Validate Session.bru` - Validate solution
10. `Sessions/Delete Session.bru` - Delete session

## Test Collection (32 requests)

**Purpose**: Automated testing, CI/CD, regression testing

**Location**: `.bruno/Tests/` folder

**Characteristics**:
- Failure scenarios included
- Auto-populated variables
- Self-contained test fixtures
- Designed for CLI execution
- Descriptive test names indicate scenario

**Test Endpoints**:

### API Keys (5 requests)
1. `Create API Key - Unauthorized.bru` - Missing master key
2. `Create API Key for Testing.bru` - Creates fixture
3. `Update API Key (Test).bru` - Uses auto-populated hash
4. `Delete API Key (Test).bru` - Uses auto-populated hash
5. `Delete API Key - Not Found.bru` - 404 scenario

### Sessions (6 requests)
1. `Create Session - Unauthorized.bru` - Missing API key
2. `Create Session - Invalid Parameters.bru` - Bad params
3. `Get Image - Not Found.bru` - Non-existent session
4. `Validate Session - Wrong Answer.bru` - Incorrect solution
5. `Validate Session - Max Attempts.bru` - Attempt tracking
6. `Validate Session - Session Deleted.bru` - Post-deletion

## Test Infrastructure

### Scripts

**Location**: `.bruno/Tests/Scripts/`

1. **test-bruno.sh** - Quick happy path test
   - 5 requests, 13 tests
   - Success scenarios only
   - ~130ms execution time
   - Use for: Smoke testing, quick validation

2. **test-bruno-full.sh** - Comprehensive test suite
   - 41 requests, 95 tests
   - Success + failure scenarios
   - ~170ms execution time
   - Use for: CI/CD, full regression testing

### Documentation

**Location**: `.bruno/Tests/Documentation/`

1. **TEST-SCENARIOS.md** - Detailed test documentation
   - All 41 test scenarios explained
   - Expected results
   - Variable management
   - Execution order

## Usage Guidelines

### When to Use Core Collection

✅ **Use for**:
- Manual API exploration in Bruno GUI
- One-off testing during development
- Demonstrating API usage to others
- Creating custom request sequences
- Learning the API

❌ **Don't use for**:
- Automated testing
- CI/CD pipelines
- Regression testing
- Failure scenario testing

### When to Use Test Collection

✅ **Use for**:
- Automated testing via CLI
- CI/CD integration
- Regression testing
- Validating all scenarios
- Failure case testing

❌ **Don't use for**:
- Interactive GUI testing
- Manual exploration
- One-off requests

## Variable Management

### Core Collection Variables
- **Manual entry required**: Update placeholders in requests
- **No auto-population**: Variables don't auto-fill
- **Environment only**: Uses `{{var}}` from environment files

### Test Collection Variables
- **Auto-populated**: Post-response scripts set variables
- **Test fixtures**: `test_key_hash` for update/delete
- **Scoped to run**: Variables only persist within one `bru run`

## Best Practices

### For Core Collection
1. Keep requests simple and clean
2. Use descriptive placeholder values
3. Document required fields in request body
4. Avoid test-specific logic
5. Make each request self-contained

### For Test Collection
1. Group related scenarios together
2. Use descriptive names indicating scenario
3. Include both success and failure cases
4. Auto-populate variables when possible
5. Clean up test fixtures after use

### For Scripts
1. Run from project root
2. Use relative paths from script location
3. Document expected environment
4. Include helpful output messages
5. Exit with proper status codes

## Migration Guide

### Moving Requests Between Collections

**Core → Tests**:
1. Move `.bru` file to appropriate Tests/ subfolder
2. Update to use auto-populated variables
3. Add test-specific logic if needed
4. Update test scripts to include request
5. Document in TEST-SCENARIOS.md

**Tests → Core**:
1. Remove test-specific logic
2. Replace auto-variables with placeholders
3. Move `.bru` file to appropriate root folder
4. Update documentation
5. Test manually in GUI

## File Naming Conventions

### Core Collection
- **Pattern**: `{Action} {Resource}.bru`
- **Examples**:
  - `Create API Key.bru`
  - `Get Image (Binary).bru`
  - `Delete Session.bru`

### Test Collection
- **Pattern**: `{Action} {Resource} - {Scenario}.bru`
- **Examples**:
  - `Create Session - Unauthorized.bru`
  - `Get Image - Not Found.bru`
  - `Delete API Key (Test).bru` (for fixtures)

## Maintenance

### Adding New Core Endpoints
1. Create `.bru` file in appropriate folder
2. Use manual placeholders
3. Update main README.md
4. Test in Bruno GUI
5. Create corresponding test scenarios

### Adding New Test Scenarios
1. Identify scenario to test (success/failure)
2. Create `.bru` file in Tests/{folder}/
3. Update test scripts
4. Update TEST-SCENARIOS.md
5. Run full test suite to verify

## Quick Reference

| Task | Command |
|------|---------|
| Quick test | `./.bruno/Tests/Scripts/test-bruno.sh` |
| Full tests | `./.bruno/Tests/Scripts/test-bruno-full.sh` |
| GUI usage | Open `.bruno/` in Bruno app |
| Core docs | `.bruno/README.md` |
| Test docs | `.bruno/Tests/README.md` |
| Scenarios | `.bruno/Tests/Documentation/TEST-SCENARIOS.md` |

## See Also

- **README.md** - Main collection documentation
- **Tests/README.md** - Test-specific documentation
- **Tests/Documentation/TEST-SCENARIOS.md** - Detailed test scenarios
