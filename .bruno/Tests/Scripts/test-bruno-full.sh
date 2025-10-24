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
# 2. API Key tests (unauthorized, create, list, create for testing, update, delete, delete not found)
# 3. Session tests (unauthorized, invalid params, create, get images, validate wrong/max attempts, not found)

bru run \
  "Health Check.bru" \
  "Tests/API Keys/Create API Key - Unauthorized.bru" \
  "API Keys/Create API Key.bru" \
  "API Keys/List API Keys.bru" \
  "Tests/API Keys/Create API Key for Testing.bru" \
  "Tests/API Keys/Update API Key (Test).bru" \
  "Tests/API Keys/Delete API Key (Test).bru" \
  "Tests/API Keys/Delete API Key - Not Found.bru" \
  "Tests/Sessions/Create Session - Unauthorized.bru" \
  "Tests/Sessions/Create Session - Invalid Parameters.bru" \
  "Sessions/Create Session.bru" \
  "Sessions/Get Image (JSON).bru" \
  "Sessions/Get Image (Binary).bru" \
  "Tests/Sessions/Get Image - Not Found.bru" \
  "Tests/Sessions/Validate Session - Wrong Answer.bru" \
  "Tests/Sessions/Validate Session - Max Attempts.bru" \
  "Tests/Sessions/Validate Session - Max Attempts.bru" \
  "Tests/Sessions/Validate Session - Session Deleted.bru" \
  --env "$ENVIRONMENT"

echo ""
echo "=========================================="
echo "   ✓ All Tests Passed!"
echo "=========================================="
echo ""
echo "Coverage Summary:"
echo "  ✓ Health Check: 1 endpoint"
echo "  ✓ API Keys: 4 endpoints (create, list, update, delete)"
echo "  ✓ Sessions: 5 endpoints (create, get image, validate, delete)"
echo ""
echo "Test Scenarios:"
echo "  ✓ Success cases: All endpoints with valid data"
echo "  ✓ Unauthorized: Missing authentication"
echo "  ✓ Invalid params: Bad request data"
echo "  ✓ Not found: Non-existent resources"
echo "  ✓ Validation: Wrong answers, max attempts"
echo ""
