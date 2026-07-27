#!/bin/bash

set -e

echo "=========================================="
echo "   CaptchAPI - Comprehensive Test Suite"
echo "=========================================="
echo ""
echo "Testing all endpoints with success and failure scenarios"
echo ""

# Change to the Bruno collection root directory
cd "$(dirname "$0")/../../"

# Detect environment (default to local)
ENVIRONMENT="${BRUNO_ENV:-local}"
echo "Using environment: $ENVIRONMENT"

# Run comprehensive test suite
# This runs all tests in the correct order:
# 1. Health check
# 2. API Key tests (unauthorized, create, list, list unauthorized, create for testing, update, update not found, update unauthorized, delete, delete not found)
# 3. Admin tests (cleanup unauthorized, invalid key, success)
# 4. Session tests (unauthorized, invalid params, create, get details, get images, validate wrong/max attempts, not found, delete unauthorized, delete not found)
# 5. Admin config tests (get/patch/reload) — LAST on purpose: PATCH mutates server-wide
#    settings, and the session tests above depend on MAX_VALIDATION_ATTEMPTS being unchanged.
#    The block ends with a reload, which discards every runtime override.

bru run \
  "Health Check.bru" \
  "Tests/API Keys/Create API Key - Unauthorized.bru" \
  "Tests/API Keys/List API Keys - Unauthorized.bru" \
  "Tests/API Keys/Update API Key - Unauthorized.bru" \
  "API Keys/Create API Key.bru" \
  "API Keys/List API Keys.bru" \
  "Tests/API Keys/Create API Key for Testing.bru" \
  "Tests/API Keys/Update API Key (Test).bru" \
  "Tests/API Keys/Update API Key - Not Found.bru" \
  "Tests/API Keys/Delete API Key (Test).bru" \
  "Tests/API Keys/Delete API Key - Not Found.bru" \
  "Tests/Admin/Cleanup - Unauthorized.bru" \
  "Tests/Admin/Cleanup - Invalid Master Key.bru" \
  "Tests/Admin/Cleanup - Success.bru" \
  "Tests/Sessions/Create Session - Unauthorized.bru" \
  "Tests/Sessions/Create Session - Invalid Parameters.bru" \
  "Tests/Sessions/Create Session - Invalid Length.bru" \
  "Tests/Sessions/Create Session - Invalid Width.bru" \
  "Tests/Sessions/Create Session - Invalid Height.bru" \
  "Sessions/Create Session.bru" \
  "Tests/Sessions/Validate Session - Unauthorized.bru" \
  "Sessions/Get Session Details.bru" \
  "Sessions/Get Image (Binary).bru" \
  "Tests/Sessions/Get Image - Not Found.bru" \
  "Tests/Sessions/Validate Session - Wrong Answer.bru" \
  "Tests/Sessions/Validate Session - Max Attempts.bru" \
  "Tests/Sessions/Validate Session - Max Attempts.bru" \
  "Tests/Sessions/Validate Session - Session Deleted.bru" \
  "Sessions/Create Session.bru" \
  "Tests/Sessions/Delete Session - Unauthorized.bru" \
  "Tests/Sessions/Delete Session - Not Found.bru" \
  "Sessions/Delete Session.bru" \
  "Tests/Admin Config/Get Config - Unauthorized.bru" \
  "Tests/Admin Config/Get Config - Invalid Master Key.bru" \
  "Tests/Admin Config/Get Config - Success.bru" \
  "Tests/Admin Config/Patch Config - Unauthorized.bru" \
  "Tests/Admin Config/Patch Config - Not Reloadable.bru" \
  "Tests/Admin Config/Patch Config - Invalid Value.bru" \
  "Tests/Admin Config/Patch Config - Success.bru" \
  "Tests/Admin Config/Reload Config - Unauthorized.bru" \
  "Tests/Admin Config/Get Stored Config.bru" \
  "Tests/Admin Config/Put Stored Config - Not Persistable.bru" \
  "Tests/Admin Config/Put Stored Config - Success.bru" \
  "Tests/Admin Config/Delete Stored Config.bru" \
  "Tests/Admin Config/Reload Config - Success.bru" \
  --env "$ENVIRONMENT"

echo ""
echo "=========================================="
echo "   ✓ All Tests Passed!"
echo "=========================================="
echo ""
echo "Coverage Summary:"
echo "  ✓ Health Check: 1 endpoint"
echo "  ✓ API Keys: 4 endpoints (create, list, update, delete)"
echo "  ✓ Admin: 4 endpoints (cleanup, get/patch config, reload config)"
echo "  ✓ Sessions: 5 endpoints (create, get details, get image, validate, delete)"
echo ""
echo "Test Scenarios:"
echo "  ✓ Success cases: All endpoints with valid data"
echo "  ✓ Unauthorized: Missing authentication (create/list/update API keys, create/validate/delete sessions, cleanup)"
echo "  ✓ Invalid params: Bad request data (difficulty, length, width, height)"
echo "  ✓ Not found: Non-existent resources (API key update, API key delete, session delete)"
echo "  ✓ Validation: Wrong answers, max attempts"
echo ""
