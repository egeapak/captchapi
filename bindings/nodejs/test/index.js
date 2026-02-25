/**
 * Comprehensive tests for @captchapi/core native module
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
    for (const suffix of ['', '-wal', '-shm']) {
      const file = TEST_DB + suffix;
      if (fs.existsSync(file)) {
        fs.unlinkSync(file);
      }
    }
  } catch (e) {
    // Ignore cleanup errors
  }
}

// Test runner
class TestRunner {
  constructor() {
    this.passed = 0;
    this.failed = 0;
    this.tests = [];
  }

  test(name, fn) {
    this.tests.push({ name, fn });
  }

  async run() {
    console.log('Running CaptchAPI Node.js tests...\n');
    console.log('='.repeat(60));

    for (const { name, fn } of this.tests) {
      try {
        await fn();
        console.log(`✓ ${name}`);
        this.passed++;
      } catch (error) {
        console.log(`✗ ${name}`);
        console.log(`  Error: ${error.message}`);
        this.failed++;
      }
    }

    console.log('='.repeat(60));
    console.log(`\nResults: ${this.passed} passed, ${this.failed} failed`);
    console.log('='.repeat(60));

    return this.failed === 0;
  }
}

async function runTests() {
  const runner = new TestRunner();
  let api;

  // ============================================
  // LIFECYCLE TESTS
  // ============================================

  runner.test('Lifecycle: Create API instance', async () => {
    cleanup();
    api = await CaptchaApi.create({
      databaseUrl: `sqlite:${TEST_DB}`,
      apiKeySalt: 'test-salt-12345',
      defaultSessionTtlSeconds: 300,
      maxSessionTtlSeconds: 3600,
      maxValidationAttempts: 3,
      runMigrations: true,
    });
    assert(api, 'API should be created');
  });

  runner.test('Lifecycle: Create second instance (same DB)', async () => {
    // Should be able to create another instance pointing to same DB
    const api2 = await CaptchaApi.create({
      databaseUrl: `sqlite:${TEST_DB}`,
      apiKeySalt: 'test-salt-12345',
      runMigrations: false, // Don't run migrations again
    });
    assert(api2, 'Second API instance should be created');
    await api2.close();
  });

  runner.test('Lifecycle: Close and reopen connection', async () => {
    await api.close();

    // Reopen
    api = await CaptchaApi.create({
      databaseUrl: `sqlite:${TEST_DB}`,
      apiKeySalt: 'test-salt-12345',
      runMigrations: false,
    });
    assert(api, 'API should be reopened');

    // Verify it works
    const session = await api.createSession({ difficulty: 3 });
    assert(session.sessionId, 'Should be able to create session after reopen');
    await api.deleteSession(session.sessionId);
  });

  runner.test('Lifecycle: Multiple close calls (idempotent)', async () => {
    await api.close();
    await api.close(); // Second close should not throw

    // Reopen for remaining tests
    api = await CaptchaApi.create({
      databaseUrl: `sqlite:${TEST_DB}`,
      apiKeySalt: 'test-salt-12345',
      runMigrations: false,
    });
  });

  // ============================================
  // CAPTCHA GENERATION TESTS
  // ============================================

  runner.test('Generate: Stateless CAPTCHA generation', async () => {
    const result = api.generate({
      difficulty: 5,
      width: 220,
      height: 120,
    });
    assert(result.solution, 'Solution should exist');
    assert(result.solution.length === 5, 'Default solution length is 5');
    assert(result.image instanceof Buffer, 'Image should be Buffer');
    assert(result.image[0] === 0xFF && result.image[1] === 0xD8, 'Should be JPEG');
  });

  runner.test('Generate: Custom length', async () => {
    const result = api.generate({ length: 8 });
    assert(result.solution.length === 8, 'Solution should be 8 characters');
    assert(result.image instanceof Buffer, 'Image should be Buffer');
  });

  runner.test('Generate: Dark mode', async () => {
    const result = api.generate({ darkMode: true, difficulty: 3 });
    assert(result.image.length > 1000, 'Dark mode image should have data');
  });

  runner.test('Generate: Various difficulty levels', async () => {
    for (const difficulty of [1, 5, 10]) {
      const result = api.generate({ difficulty });
      assert(result.solution, `Difficulty ${difficulty} should work`);
    }
  });

  // ============================================
  // SESSION TESTS
  // ============================================

  runner.test('Session: Create with defaults', async () => {
    const session = await api.createSession();
    assert(session.sessionId, 'Session ID should exist');
    assert(session.createdAt > 0, 'Created timestamp should exist');
    assert(session.expiresAt > session.createdAt, 'Should have expiry');
    assert(session.image instanceof Buffer, 'Image should be Buffer');
    await api.deleteSession(session.sessionId);
  });

  runner.test('Session: Create with custom options', async () => {
    const session = await api.createSession({
      difficulty: 7,
      width: 300,
      height: 150,
      expiresInSeconds: 600,
      darkMode: true,
    });
    assert(session.sessionId);

    const info = await api.getSession(session.sessionId);
    assert.strictEqual(info.difficulty, 7);
    assert.strictEqual(info.width, 300);
    assert.strictEqual(info.height, 150);
    assert.strictEqual(info.darkMode, true);

    await api.deleteSession(session.sessionId);
  });

  runner.test('Session: Get image separately', async () => {
    const session = await api.createSession({ difficulty: 3 });
    const image = await api.getImage(session.sessionId);
    assert(image instanceof Buffer);
    assert(image[0] === 0xFF && image[1] === 0xD8, 'Should be JPEG');
    await api.deleteSession(session.sessionId);
  });

  runner.test('Session: Delete returns true for existing', async () => {
    const session = await api.createSession();
    const deleted = await api.deleteSession(session.sessionId);
    assert.strictEqual(deleted, true);
  });

  runner.test('Session: Delete returns false for non-existing', async () => {
    const deleted = await api.deleteSession('non-existent-id');
    assert.strictEqual(deleted, false);
  });

  // ============================================
  // VALIDATION TESTS
  // ============================================

  runner.test('Validation: Correct solution (case-sensitive)', async () => {
    const session = await api.createSession();
    const result = await api.validate(session.sessionId, session.text);
    assert.strictEqual(result.valid, true);
    assert.strictEqual(result.attemptsRemaining, 0);
  });

  runner.test('Validation: Session deleted after success', async () => {
    const session = await api.createSession();
    await api.validate(session.sessionId, session.text);

    try {
      await api.getSession(session.sessionId);
      assert.fail('Should have thrown');
    } catch (e) {
      assert(e.message.includes('not found'), 'Should be not found error');
    }
  });

  runner.test('Validation: Wrong solution decrements attempts', async () => {
    const session = await api.createSession();

    const result1 = await api.validate(session.sessionId, 'DEFINITELY_WRONG_ANSWER');
    assert.strictEqual(result1.valid, false);
    assert.strictEqual(result1.attemptsRemaining, 2);

    const result2 = await api.validate(session.sessionId, 'STILL_WRONG_ANSWER');
    assert.strictEqual(result2.attemptsRemaining, 1);

    await api.deleteSession(session.sessionId);
  });

  runner.test('Validation: Max attempts exhausted', async () => {
    const session = await api.createSession();

    // Exhaust all 3 attempts
    await api.validate(session.sessionId, 'WRONG1');
    await api.validate(session.sessionId, 'WRONG2');
    const result = await api.validate(session.sessionId, 'WRONG3');

    assert.strictEqual(result.valid, false);
    assert.strictEqual(result.attemptsRemaining, 0);

    // Session should be deleted
    try {
      await api.getSession(session.sessionId);
      assert.fail('Should have thrown');
    } catch (e) {
      // Expected
    }
  });

  // ============================================
  // API KEY TESTS
  // ============================================

  runner.test('API Key: Create and validate', async () => {
    const key = await api.createApiKey('Test Key');
    assert(key.apiKey, 'Should have API key');
    assert(key.keyHash, 'Should have key hash');

    const valid = await api.validateApiKey(key.apiKey);
    assert.strictEqual(valid, true);

    await api.deleteApiKey(key.keyHash);
  });

  runner.test('API Key: Invalid key returns false', async () => {
    const valid = await api.validateApiKey('invalid-key-12345');
    assert.strictEqual(valid, false);
  });

  runner.test('API Key: List and get', async () => {
    const key = await api.createApiKey('List Test');

    const keys = await api.listApiKeys();
    assert(Array.isArray(keys));
    assert(keys.some(k => k.keyHash === key.keyHash));

    const info = await api.getApiKey(key.keyHash);
    assert(info);
    assert.strictEqual(info.description, 'List Test');
    assert.strictEqual(info.isActive, true);

    await api.deleteApiKey(key.keyHash);
  });

  runner.test('API Key: Update', async () => {
    const key = await api.createApiKey('Original');

    await api.updateApiKey(key.keyHash, false, 'Updated');

    const info = await api.getApiKey(key.keyHash);
    assert.strictEqual(info.description, 'Updated');
    assert.strictEqual(info.isActive, false);

    await api.deleteApiKey(key.keyHash);
  });

  // ============================================
  // ERROR HANDLING TESTS
  // ============================================

  runner.test('Error: Get non-existent session', async () => {
    try {
      await api.getSession('does-not-exist');
      assert.fail('Should have thrown');
    } catch (e) {
      assert(e.message.includes('not found'), 'Should indicate not found');
    }
  });

  runner.test('Error: Get image for non-existent session', async () => {
    try {
      await api.getImage('does-not-exist');
      assert.fail('Should have thrown');
    } catch (e) {
      assert(e.message.includes('not found'));
    }
  });

  runner.test('Error: Validate non-existent session', async () => {
    try {
      await api.validate('does-not-exist', 'test');
      assert.fail('Should have thrown');
    } catch (e) {
      assert(e.message.includes('not found'));
    }
  });

  runner.test('Error: Invalid difficulty (too low)', async () => {
    try {
      await api.createSession({ difficulty: 0 });
      assert.fail('Should have thrown');
    } catch (e) {
      assert(e.message.includes('difficulty'));
    }
  });

  runner.test('Error: Invalid difficulty (too high)', async () => {
    try {
      await api.createSession({ difficulty: 11 });
      assert.fail('Should have thrown');
    } catch (e) {
      assert(e.message.includes('difficulty'));
    }
  });

  runner.test('Error: Invalid width', async () => {
    try {
      await api.createSession({ width: 10 }); // Too small (min: 50)
      assert.fail('Should have thrown');
    } catch (e) {
      assert(e.message.includes('width'));
    }
  });

  runner.test('Error: Invalid height', async () => {
    try {
      await api.createSession({ height: 10 }); // Too small (min: 30)
      assert.fail('Should have thrown');
    } catch (e) {
      assert(e.message.includes('height'));
    }
  });

  runner.test('Error: TTL exceeds maximum', async () => {
    try {
      await api.createSession({ expiresInSeconds: 99999 });
      assert.fail('Should have thrown');
    } catch (e) {
      assert(e.message.includes('expires_in_seconds') || e.message.includes('cannot exceed'));
    }
  });

  runner.test('Error: createApiKey with empty description throws', async () => {
    try {
      await api.createApiKey('');
      assert.fail('Should have thrown');
    } catch (e) {
      assert(e.message.toLowerCase().includes('description') || e.message.toLowerCase().includes('empty'));
    }
  });

  runner.test('Error: updateApiKey with empty description throws', async () => {
    const key = await api.createApiKey('Valid key');
    try {
      await api.updateApiKey(key.keyHash, true, '');
      assert.fail('Should have thrown');
    } catch (e) {
      assert(e.message.toLowerCase().includes('description') || e.message.toLowerCase().includes('empty'));
    } finally {
      await api.deleteApiKey(key.keyHash);
    }
  });

  // ============================================
  // CLEANUP TESTS
  // ============================================

  runner.test('Cleanup: Remove expired sessions', async () => {
    const count = await api.cleanupExpired();
    assert(typeof count === 'number');
  });

  // ============================================
  // FINAL CLEANUP
  // ============================================

  runner.test('Final: Close connection', async () => {
    await api.close();
  });

  // Run all tests
  const success = await runner.run();
  cleanup();
  process.exit(success ? 0 : 1);
}

runTests().catch(err => {
  console.error('Test runner failed:', err);
  cleanup();
  process.exit(1);
});
