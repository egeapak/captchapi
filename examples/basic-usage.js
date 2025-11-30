/**
 * Basic usage example for @captchapi/core
 *
 * This example demonstrates how to:
 * 1. Create a CaptchaApi instance
 * 2. Generate and display a CAPTCHA
 * 3. Validate user input
 * 4. Handle API keys
 */

const { CaptchaApi } = require('../index.js');
const fs = require('fs');
const path = require('path');
const readline = require('readline');

// Create readline interface for user input
const rl = readline.createInterface({
  input: process.stdin,
  output: process.stdout,
});

function question(prompt) {
  return new Promise((resolve) => {
    rl.question(prompt, resolve);
  });
}

async function main() {
  console.log('CaptchAPI - Basic Usage Example\n');
  console.log('='.repeat(50));

  // Initialize the API
  console.log('\n1. Initializing CaptchaApi...');
  const api = await CaptchaApi.create({
    databaseUrl: 'sqlite:./example-captcha.db',
    apiKeySalt: 'example-salt-please-change-in-production',
    defaultSessionTtlSeconds: 300,
    maxValidationAttempts: 3,
  });
  console.log('   ✓ API initialized\n');

  // Create a CAPTCHA session
  console.log('2. Creating CAPTCHA session...');
  const session = await api.createSession({
    difficulty: 5,
    width: 220,
    height: 120,
    darkMode: false,
  });

  // Save the image to a file
  const imagePath = path.join(__dirname, 'captcha.jpg');
  fs.writeFileSync(imagePath, session.image);
  console.log(`   ✓ Session created: ${session.sessionId}`);
  console.log(`   ✓ Image saved to: ${imagePath}`);
  console.log(`   ✓ Expires at: ${new Date(session.expiresAt).toISOString()}\n`);

  // Get session info
  const info = await api.getSession(session.sessionId);
  console.log('3. Session info:');
  console.log(`   - Difficulty: ${info.difficulty}`);
  console.log(`   - Dimensions: ${info.width}x${info.height}`);
  console.log(`   - Attempts: ${info.attemptCount}/${3}\n`);

  // Ask user to solve the CAPTCHA
  console.log('4. Please open the captcha.jpg file and enter the text you see.');
  console.log('   (You have 3 attempts)\n');

  let solved = false;
  while (!solved) {
    const answer = await question('   Enter CAPTCHA text: ');

    try {
      const result = await api.validate(session.sessionId, answer);

      if (result.valid) {
        console.log('\n   ✓ CAPTCHA solved correctly!\n');
        solved = true;
      } else if (result.attemptsRemaining > 0) {
        console.log(`   ✗ Incorrect. ${result.attemptsRemaining} attempts remaining.\n`);
      } else {
        console.log('\n   ✗ Maximum attempts reached. Session expired.\n');
        break;
      }
    } catch (error) {
      console.log(`\n   ✗ Error: ${error.message}\n`);
      break;
    }
  }

  // API Key management example
  console.log('5. API Key Management:');

  // Create an API key
  const keyResult = await api.createApiKey('Example API Key');
  console.log(`   ✓ Created API key: ${keyResult.apiKey}`);
  console.log(`   ✓ Key hash: ${keyResult.keyHash.substring(0, 32)}...`);

  // Validate the key
  const isValid = await api.validateApiKey(keyResult.apiKey);
  console.log(`   ✓ Key is valid: ${isValid}`);

  // List all keys
  const keys = await api.listApiKeys();
  console.log(`   ✓ Total API keys: ${keys.length}`);

  // Clean up
  await api.deleteApiKey(keyResult.keyHash);
  console.log('   ✓ Cleaned up test API key\n');

  // Stateless generation example
  console.log('6. Stateless CAPTCHA generation:');
  const generated = api.generate({
    text: 'HELLO',
    difficulty: 3,
  });
  console.log(`   ✓ Generated CAPTCHA with solution: ${generated.solution}`);
  console.log(`   ✓ Image size: ${generated.image.length} bytes\n`);

  // Close the connection
  await api.close();
  console.log('7. Connection closed.\n');

  // Cleanup
  fs.unlinkSync(imagePath);
  fs.unlinkSync('./example-captcha.db');
  try {
    fs.unlinkSync('./example-captcha.db-wal');
    fs.unlinkSync('./example-captcha.db-shm');
  } catch (e) {
    // Ignore if WAL files don't exist
  }

  rl.close();
  console.log('Example completed!');
}

main().catch((error) => {
  console.error('Error:', error);
  rl.close();
  process.exit(1);
});
