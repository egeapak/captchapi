#!/bin/bash

set -e

echo "===== Running Bruno Quick Test (Happy Path) ====="
echo ""
echo "Running core workflow: Health → Create API Key → Create Session → Get Images"
echo ""

# Change to the Bruno collection root directory
cd "$(dirname "$0")/../../"

# Detect environment (default to local)
ENVIRONMENT="${BRUNO_ENV:-local}"
echo "Using environment: $ENVIRONMENT"

# Run all requests in a single command to preserve environment variables
# The post-response scripts in "Create API Key" and "Create Session"
# will set api_key and session_id variables for subsequent requests
bru run \
  "Health Check.bru" \
  "API Keys/Create API Key.bru" \
  "API Keys/List API Keys.bru" \
  "Sessions/Create Session.bru" \
  "Sessions/Get Image (Binary).bru" \
  --env "$ENVIRONMENT"

echo ""
echo "===== Quick Test Passed! ====="
echo ""
echo "To run comprehensive tests with all scenarios:"
echo "  ./.bruno/Tests/Scripts/test-bruno-full.sh"
echo ""
