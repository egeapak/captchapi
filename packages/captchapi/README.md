# captchapi

High-performance CAPTCHA generation and validation for Node.js, powered by Rust.

## Installation

```bash
npm install captchapi
```

## Quick Start

```typescript
import { Captchapi, CaptchaApiBuilder } from 'captchapi';

// Using the builder (recommended)
const api = await CaptchaApiBuilder.create()
  .database('./captcha.db')
  .salt('your-secret-salt')
  .sessionTtl(300)
  .maxAttempts(3)
  .build();

// Create a CAPTCHA session
const session = await api.createSession({
  length: 5,      // CAPTCHA text length (1-20)
  difficulty: 5,
  width: 220,
  height: 120,
});

console.log('Session ID:', session.sessionId);
console.log('CAPTCHA text:', session.text);
console.log('Image size:', session.image.length, 'bytes');

// Validate user input
const result = await api.validate(session.sessionId, userAnswer);
if (result.valid) {
  console.log('CAPTCHA solved!');
} else {
  console.log('Wrong answer. Attempts remaining:', result.attemptsRemaining);
}

// Clean up
await api.close();
```

## Features

- **High Performance**: Native Rust bindings via NAPI-RS
- **Type Safe**: Full TypeScript support with detailed types
- **Flexible**: Builder pattern, singleton support, lifecycle callbacks
- **Debug Mode**: Enable with `DEBUG=captchapi` or `CAPTCHAPI_DEBUG=1`
- **Error Handling**: Rich error types with helpful messages
- **Dual Module**: Supports both ESM and CommonJS

## API

### Factory Methods

```typescript
// Using builder (recommended)
const api = await CaptchaApiBuilder.create()
  .database('./captcha.db')
  .salt('your-secret-salt')
  .build();

// Direct creation
const api = await Captchapi.create({
  databaseUrl: 'sqlite:./captcha.db',
  apiKeySalt: 'your-secret-salt',
});

// Singleton pattern
const api = await Captchapi.getInstance(config);

// Quick start (development only)
const api = await quickStart('./captcha.db');

// In-memory (testing)
const api = await inMemory();
```

### Session Management

```typescript
// Create session
const session = await api.createSession({
  length: 5,                // Optional: CAPTCHA text length 1-20 (default: 5)
  difficulty: 5,            // 1-10
  width: 220,               // pixels
  height: 120,              // pixels
  darkMode: false,          // dark background
  compression: 40,          // JPEG quality 1-100
  expiresInSeconds: 300,    // TTL
});

// Response includes generated text
console.log('Generated text:', session.text);

// Get image
const imageBuffer = await api.getImage(session.sessionId);

// Get session info
const info = await api.getSession(session.sessionId);

// Validate solution (case-sensitive)
const result = await api.validate(session.sessionId, userAnswer);
// Note: Validation is case-sensitive - "aBc5X" !== "abc5x"

// Delete session
await api.deleteSession(session.sessionId);

// Cleanup expired
const count = await api.cleanupExpired();
```

### Stateless Generation

```typescript
// Generate without storing
const { solution, image } = api.generate({
  difficulty: 5,
  width: 220,
  height: 120,
});
// You manage storage yourself
```

### API Keys

```typescript
// Create
const { apiKey, keyHash } = await api.createApiKey('My API Key');

// Validate
const isValid = await api.validateApiKey(apiKey);

// List
const keys = await api.listApiKeys();

// Update
await api.updateApiKey(keyHash, true, 'New description');

// Delete
await api.deleteApiKey(keyHash);
```

### Lifecycle Callbacks

```typescript
const api = await Captchapi.create(config, {
  callbacks: {
    onInitialized: (api) => console.log('Ready!'),
    onClosed: () => console.log('Closed'),
    onSessionCreated: (id) => console.log('Session:', id),
    onSessionValidated: (id, valid) => console.log('Valid:', valid),
    onSessionDeleted: (id) => console.log('Deleted:', id),
    onError: (err) => console.error('Error:', err),
  }
});
```

## Configuration

### Builder Options

```typescript
CaptchaApiBuilder.create()
  .database('./captcha.db')      // Required: SQLite path
  .salt('your-secret-salt')      // Required: API key salt
  .sessionTtl(300)               // Default session TTL (seconds)
  .maxSessionTtl(3600)           // Max allowed TTL
  .maxAttempts(3)                // Max validation attempts
  .migrations(true)              // Run migrations on start
  .skipCompatibilityCheck()      // Skip version check
  .build();
```

### Environment Variables

```bash
# Enable debug logging
DEBUG=captchapi
# or
CAPTCHAPI_DEBUG=1
```

## Error Handling

```typescript
import {
  CaptchapiError,
  NativeBindingError,
  VersionCompatibilityError,
  ConfigurationError,
  SessionError,
} from 'captchapi';

try {
  await api.validate(sessionId, answer);
} catch (error) {
  if (error instanceof SessionError) {
    console.log('Session error:', error.sessionId);
  } else if (error instanceof NativeBindingError) {
    console.log('Native module failed to load');
  }
}
```

## Utilities

```typescript
import {
  VERSION,
  getCoreVersion,
  isCompatibleCoreVersion,
  getEnvironment,
} from 'captchapi';

console.log('Wrapper version:', VERSION);
console.log('Core version:', getCoreVersion());
console.log('Compatible:', isCompatibleCoreVersion());
console.log('Environment:', getEnvironment());
```

## Supported Platforms

| Platform | Architecture | Status |
|----------|-------------|--------|
| Windows  | x64         | ✅     |
| macOS    | x64         | ✅     |
| macOS    | ARM64       | ✅     |
| Linux    | x64 (glibc) | ✅     |
| Linux    | x64 (musl)  | ✅     |
| Linux    | ARM64       | ✅     |

## License

MIT
