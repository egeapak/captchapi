/**
 * Tests for @captchapi/core native module
 *
 * Run with: npm test
 * Requires the native module to be built first: npm run build
 */

const { CaptchaApi } = require('../index.js');
const fs = require('fs');
const path = require('path');
const assert = require('assert');

// Test database path
const TEST_DB = path.join(__dirname, 'test-captcha.db');

// Cleanup function
function cleanup() {
  try {
    if (fs.existsSync(TEST_DB)) {
      fs.unlinkSync(TEST_DB);
    }
    if (fs.existsSync(TEST_DB + '-wal')) {
      fs.unlinkSync(TEST_DB + '-wal');
    }
    if (fs.existsSync(TEST_DB + '-shm')) {
      fs.unlinkSync(TEST_DB + '-shm');
    }
  } catch (e) {
    // Ignore cleanup errors
  }
}

async function runTests() {
  console.log('Running CaptchAPI Node.js tests...\n');

  let api;
  let passed = 0;
  let failed = 0;

  try {
    // Clean up before tests
    cleanup();

    // Test 1: Create API instance
    console.log('Test 1: Create CaptchaApi instance');
    api = await CaptchaApi.create({
      databaseUrl: `sqlite:${TEST_DB}`,
      apiKeySalt: 'test-salt-12345',
      defaultSessionTtlSeconds: 300,
      maxSessionTtlSeconds: 3600,
      maxValidationAttempts: 3,
      runMigrations: true,
    });
    console.log('  ✓ CaptchaApi created successfully\n');
    passed++;

    // Test 2: Generate CAPTCHA (stateless)
    console.log('Test 2: Generate CAPTCHA (stateless)');
    const generated = api.generate({
      difficulty: 5,
      width: 220,
      height: 120,
      darkMode: false,
    });
    assert(generated.solution, 'Solution should exist');
    assert(generated.solution.length === 5, 'Solution should be 5 characters');
    assert(generated.image instanceof Buffer, 'Image should be a Buffer');
    assert(generated.image.length > 1000, 'Image should have substantial data');
    // Check JPEG signature
    assert(generated.image[0] === 0xFF && generated.image[1] === 0xD8, 'Image should be JPEG');
    console.log(`  ✓ Generated CAPTCHA: solution="${generated.solution}", image=${generated.image.length} bytes\n`);
    passed++;

    // Test 3: Create session
    console.log('Test 3: Create CAPTCHA session');
    const session = await api.createSession({
      difficulty: 5,
      width: 220,
      height: 120,
      expiresInSeconds: 300,
    });
    assert(session.sessionId, 'Session ID should exist');
    assert(session.createdAt > 0, 'Created timestamp should exist');
    assert(session.expiresAt > session.createdAt, 'Expires should be after created');
    assert(session.image instanceof Buffer, 'Image should be a Buffer');
    console.log(`  ✓ Session created: id="${session.sessionId}"\n`);
    passed++;

    // Test 4: Get session info
    console.log('Test 4: Get session info');
    const info = await api.getSession(session.sessionId);
    assert(info.sessionId === session.sessionId, 'Session ID should match');
    assert(info.difficulty === 5, 'Difficulty should match');
    assert(info.width === 220, 'Width should match');
    assert(info.height === 120, 'Height should match');
    assert(info.attemptCount === 0, 'Attempt count should be 0');
    console.log(`  ✓ Session info retrieved: attempts=${info.attemptCount}\n`);
    passed++;

    // Test 5: Get session image
    console.log('Test 5: Get session image');
    const image = await api.getImage(session.sessionId);
    assert(image instanceof Buffer, 'Image should be a Buffer');
    assert(image.length > 1000, 'Image should have substantial data');
    console.log(`  ✓ Image retrieved: ${image.length} bytes\n`);
    passed++;

    // Test 6: Validate with wrong solution
    console.log('Test 6: Validate with wrong solution');
    const wrongResult = await api.validate(session.sessionId, 'WRONG');
    assert(wrongResult.valid === false, 'Wrong solution should be invalid');
    assert(wrongResult.attemptsRemaining >= 0, 'Should have attempts remaining info');
    console.log(`  ✓ Wrong solution rejected, attempts remaining: ${wrongResult.attemptsRemaining}\n`);
    passed++;

    // Test 7: Create and validate with correct solution
    console.log('Test 7: Validate with correct solution');
    const session2 = await api.createSession({
      text: 'TEST123',
      difficulty: 3,
    });
    const correctResult = await api.validate(session2.sessionId, 'TEST123');
    assert(correctResult.valid === true, 'Correct solution should be valid');
    console.log(`  ✓ Correct solution accepted\n`);
    passed++;

    // Test 8: Session should be deleted after successful validation
    console.log('Test 8: Session deleted after validation');
    try {
      await api.getSession(session2.sessionId);
      console.log('  ✗ Session should have been deleted\n');
      failed++;
    } catch (e) {
      console.log('  ✓ Session was deleted after successful validation\n');
      passed++;
    }

    // Test 9: API key management
    console.log('Test 9: Create API key');
    const keyResult = await api.createApiKey('Test API key');
    assert(keyResult.apiKey, 'API key should exist');
    assert(keyResult.keyHash, 'Key hash should exist');
    console.log(`  ✓ API key created: hash="${keyResult.keyHash.substring(0, 16)}..."\n`);
    passed++;

    // Test 10: Validate API key
    console.log('Test 10: Validate API key');
    const isValid = await api.validateApiKey(keyResult.apiKey);
    assert(isValid === true, 'API key should be valid');
    console.log('  ✓ API key validated successfully\n');
    passed++;

    // Test 11: List API keys
    console.log('Test 11: List API keys');
    const keys = await api.listApiKeys();
    assert(Array.isArray(keys), 'Keys should be an array');
    assert(keys.length >= 1, 'Should have at least 1 key');
    console.log(`  ✓ Listed ${keys.length} API key(s)\n`);
    passed++;

    // Test 12: Delete session
    console.log('Test 12: Delete session');
    const deleted = await api.deleteSession(session.sessionId);
    assert(deleted === true, 'Session should be deleted');
    console.log('  ✓ Session deleted\n');
    passed++;

    // Test 13: Delete API key
    console.log('Test 13: Delete API key');
    const keyDeleted = await api.deleteApiKey(keyResult.keyHash);
    assert(keyDeleted === true, 'API key should be deleted');
    console.log('  ✓ API key deleted\n');
    passed++;

    // Test 14: Cleanup expired sessions
    console.log('Test 14: Cleanup expired sessions');
    const cleanedUp = await api.cleanupExpired();
    assert(typeof cleanedUp === 'number', 'Cleanup should return a number');
    console.log(`  ✓ Cleaned up ${cleanedUp} expired session(s)\n`);
    passed++;

    // Test 15: Close connection
    console.log('Test 15: Close connection');
    await api.close();
    console.log('  ✓ Connection closed\n');
    passed++;

  } catch (error) {
    console.error('\n✗ Test failed:', error.message);
    console.error(error.stack);
    failed++;
  } finally {
    // Clean up
    cleanup();
  }

  // Summary
  console.log('='.repeat(50));
  console.log(`Tests completed: ${passed} passed, ${failed} failed`);
  console.log('='.repeat(50));

  process.exit(failed > 0 ? 1 : 0);
}

runTests();
