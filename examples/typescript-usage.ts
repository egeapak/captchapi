/**
 * TypeScript usage example for @captchapi/core
 *
 * This example demonstrates type-safe usage with full TypeScript support.
 *
 * To run:
 *   npx ts-node examples/typescript-usage.ts
 */

import {
  CaptchaApi,
  CaptchaConfig,
  CreateSessionOptions,
  SessionResult,
  ValidationResult,
  GenerateOptions,
} from '../index';
import * as fs from 'fs';

async function main(): Promise<void> {
  console.log('CaptchAPI - TypeScript Example\n');

  // Configuration with full type checking
  const config: CaptchaConfig = {
    databaseUrl: 'sqlite:./ts-example.db',
    apiKeySalt: 'typescript-example-salt',
    defaultSessionTtlSeconds: 300,
    maxSessionTtlSeconds: 3600,
    maxValidationAttempts: 3,
    runMigrations: true,
  };

  // Create API instance
  const api: CaptchaApi = await CaptchaApi.create(config);
  console.log('✓ CaptchaApi initialized\n');

  // Session options with type safety
  const sessionOptions: CreateSessionOptions = {
    difficulty: 5,
    width: 220,
    height: 120,
    darkMode: false,
    compression: 40,
  };

  // Create a session
  const session: SessionResult = await api.createSession(sessionOptions);
  console.log('✓ Session created:');
  console.log(`  ID: ${session.sessionId}`);
  console.log(`  Created: ${new Date(session.createdAt).toISOString()}`);
  console.log(`  Expires: ${new Date(session.expiresAt).toISOString()}`);
  console.log(`  Image: ${session.image.length} bytes\n`);

  // Save image (Buffer type is automatically inferred)
  fs.writeFileSync('captcha-ts.jpg', session.image);
  console.log('✓ Image saved to captcha-ts.jpg\n');

  // Create a session with known text for validation demo
  const testSession = await api.createSession({
    text: 'DEMO',
    difficulty: 3,
  });

  // Validate with type-safe result
  const result: ValidationResult = await api.validate(
    testSession.sessionId,
    'DEMO'
  );
  console.log('✓ Validation result:');
  console.log(`  Valid: ${result.valid}`);
  console.log(`  Session ID: ${result.sessionId}`);
  console.log(`  Attempts remaining: ${result.attemptsRemaining}\n`);

  // Stateless generation with options
  const generateOptions: GenerateOptions = {
    text: 'TYPESCRIPT',
    difficulty: 7,
    width: 300,
    height: 100,
    darkMode: true,
  };

  const generated = api.generate(generateOptions);
  console.log('✓ Stateless generation:');
  console.log(`  Solution: ${generated.solution}`);
  console.log(`  Image: ${generated.image.length} bytes\n`);

  // API key management with full typing
  const apiKey = await api.createApiKey('TypeScript test key');
  console.log('✓ API key created:');
  console.log(`  Key: ${apiKey.apiKey}`);
  console.log(`  Hash: ${apiKey.keyHash.substring(0, 16)}...\n`);

  // List keys
  const keys = await api.listApiKeys();
  console.log(`✓ Found ${keys.length} API key(s)`);
  for (const key of keys) {
    console.log(`  - ${key.description || 'No description'} (active: ${key.isActive})`);
  }
  console.log();

  // Cleanup
  await api.deleteApiKey(apiKey.keyHash);
  await api.deleteSession(session.sessionId);
  await api.cleanupExpired();
  await api.close();

  // Remove test files
  fs.unlinkSync('captcha-ts.jpg');
  fs.unlinkSync('./ts-example.db');
  try {
    fs.unlinkSync('./ts-example.db-wal');
    fs.unlinkSync('./ts-example.db-shm');
  } catch {
    // Ignore
  }

  console.log('✓ Cleanup complete\n');
  console.log('TypeScript example completed successfully!');
}

main().catch((error) => {
  console.error('Error:', error);
  process.exit(1);
});
