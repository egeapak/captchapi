/**
 * CaptchAPI - High-performance CAPTCHA generation and validation for Node.js
 *
 * @packageDocumentation
 */

// =============================================================================
// Types - Re-exported from @captchapi/core
// =============================================================================

export type {
  CaptchaConfig,
  CreateSessionOptions,
  SessionResult,
  SessionInfo,
  ValidationResult,
  ApiKeyInfo,
  CreateApiKeyResult,
  GenerateOptions,
  GenerateResult,
} from "@captchapi/core";

import type {
  CaptchaConfig,
  CreateSessionOptions,
  SessionResult,
  SessionInfo,
  ValidationResult,
  ApiKeyInfo,
  CreateApiKeyResult,
  GenerateOptions,
  GenerateResult,
} from "@captchapi/core";

// =============================================================================
// Version & Compatibility
// =============================================================================

/** Current wrapper package version */
export const VERSION = "0.1.2";

/** Minimum compatible @captchapi/core version */
export const MIN_CORE_VERSION = "0.1.0";

/** Maximum compatible @captchapi/core version (exclusive) */
export const MAX_CORE_VERSION = "1.0.0";

/**
 * Check if the installed @captchapi/core version is compatible
 * @returns true if compatible, false otherwise
 */
export function isCompatibleCoreVersion(): boolean {
  try {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const corePkg = require("@captchapi/core/package.json");
    const version = corePkg.version;
    return (
      compareVersions(version, MIN_CORE_VERSION) >= 0 &&
      compareVersions(version, MAX_CORE_VERSION) < 0
    );
  } catch {
    return false;
  }
}

/**
 * Get the installed @captchapi/core version
 * @returns version string or null if not installed
 */
export function getCoreVersion(): string | null {
  try {
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const corePkg = require("@captchapi/core/package.json");
    return corePkg.version;
  } catch {
    return null;
  }
}

function compareVersions(a: string, b: string): number {
  const partsA = a.split(".").map(Number);
  const partsB = b.split(".").map(Number);

  for (let i = 0; i < Math.max(partsA.length, partsB.length); i++) {
    const numA = partsA[i] || 0;
    const numB = partsB[i] || 0;
    if (numA > numB) return 1;
    if (numA < numB) return -1;
  }
  return 0;
}

// =============================================================================
// Debug & Logging
// =============================================================================

const DEBUG =
  process.env.DEBUG?.includes("captchapi") ||
  process.env.CAPTCHAPI_DEBUG === "1" ||
  process.env.CAPTCHAPI_DEBUG === "true";

function debug(message: string, ...args: unknown[]): void {
  if (DEBUG) {
    console.log(`[captchapi] ${message}`, ...args);
  }
}

function debugError(message: string, error: unknown): void {
  if (DEBUG) {
    console.error(`[captchapi] ${message}`, error);
  }
}

// =============================================================================
// Environment Detection
// =============================================================================

export interface EnvironmentInfo {
  /** Node.js version */
  nodeVersion: string;
  /** Operating system platform */
  platform: NodeJS.Platform;
  /** CPU architecture */
  arch: string;
  /** Whether running in production mode */
  isProduction: boolean;
  /** Whether debug mode is enabled */
  isDebug: boolean;
  /** Whether native bindings are available */
  hasNativeBindings: boolean;
}

/**
 * Get information about the current runtime environment
 */
export function getEnvironment(): EnvironmentInfo {
  let hasNativeBindings = false;
  try {
    require("@captchapi/core");
    hasNativeBindings = true;
  } catch {
    hasNativeBindings = false;
  }

  return {
    nodeVersion: process.version,
    platform: process.platform,
    arch: process.arch,
    isProduction: process.env.NODE_ENV === "production",
    isDebug: DEBUG,
    hasNativeBindings,
  };
}

// =============================================================================
// Error Types
// =============================================================================

/**
 * Base error class for CaptchAPI errors
 */
export class CaptchapiError extends Error {
  constructor(
    message: string,
    public readonly code: string,
    public readonly cause?: Error
  ) {
    super(message);
    this.name = "CaptchapiError";
    Error.captureStackTrace?.(this, this.constructor);
  }
}

/**
 * Error thrown when native bindings fail to load
 */
export class NativeBindingError extends CaptchapiError {
  constructor(message: string, cause?: Error) {
    super(message, "NATIVE_BINDING_ERROR", cause);
    this.name = "NativeBindingError";
  }
}

/**
 * Error thrown when version compatibility check fails
 */
export class VersionCompatibilityError extends CaptchapiError {
  constructor(
    public readonly installedVersion: string | null,
    public readonly requiredRange: string
  ) {
    super(
      `Incompatible @captchapi/core version: ${installedVersion ?? "not installed"}. Required: ${requiredRange}`,
      "VERSION_INCOMPATIBLE"
    );
    this.name = "VersionCompatibilityError";
  }
}

/**
 * Error thrown when configuration is invalid
 */
export class ConfigurationError extends CaptchapiError {
  constructor(message: string) {
    super(message, "CONFIGURATION_ERROR");
    this.name = "ConfigurationError";
  }
}

/**
 * Error thrown when a session operation fails
 */
export class SessionError extends CaptchapiError {
  constructor(
    message: string,
    public readonly sessionId?: string,
    cause?: Error
  ) {
    super(message, "SESSION_ERROR", cause);
    this.name = "SessionError";
  }
}

// =============================================================================
// Configuration Builder
// =============================================================================

/**
 * Fluent configuration builder for CaptchaApi
 *
 * @example
 * ```typescript
 * const api = await CaptchaApiBuilder.create()
 *   .database('./captcha.db')
 *   .salt('your-secret-salt')
 *   .sessionTtl(600)
 *   .maxAttempts(5)
 *   .build();
 * ```
 */
export class CaptchaApiBuilder {
  private config: Partial<CaptchaConfig> = {};
  private skipVersionCheck = false;

  private constructor() {}

  /**
   * Create a new builder instance
   */
  static create(): CaptchaApiBuilder {
    return new CaptchaApiBuilder();
  }

  /**
   * Set the database URL
   * @param url SQLite database URL (e.g., "sqlite:./captcha.db" or just "./captcha.db")
   */
  database(url: string): this {
    // Auto-add sqlite: prefix if not present
    this.config.databaseUrl = url.startsWith("sqlite:") ? url : `sqlite:${url}`;
    return this;
  }

  /**
   * Set the API key salt
   * @param salt Secret salt for hashing API keys
   */
  salt(salt: string): this {
    this.config.apiKeySalt = salt;
    return this;
  }

  /**
   * Set the default session TTL
   * @param seconds TTL in seconds
   */
  sessionTtl(seconds: number): this {
    this.config.defaultSessionTtlSeconds = seconds;
    return this;
  }

  /**
   * Set the maximum session TTL
   * @param seconds Maximum TTL in seconds
   */
  maxSessionTtl(seconds: number): this {
    this.config.maxSessionTtlSeconds = seconds;
    return this;
  }

  /**
   * Set the maximum validation attempts per session
   * @param attempts Maximum attempts
   */
  maxAttempts(attempts: number): this {
    this.config.maxValidationAttempts = attempts;
    return this;
  }

  /**
   * Enable or disable automatic migrations
   * @param run Whether to run migrations on startup
   */
  migrations(run: boolean): this {
    this.config.runMigrations = run;
    return this;
  }

  /**
   * Skip version compatibility check (not recommended)
   */
  skipCompatibilityCheck(): this {
    this.skipVersionCheck = true;
    return this;
  }

  /**
   * Build and create the CaptchaApi instance
   * @throws {ConfigurationError} If required configuration is missing
   * @throws {VersionCompatibilityError} If @captchapi/core version is incompatible
   */
  async build(): Promise<Captchapi> {
    if (!this.config.databaseUrl) {
      throw new ConfigurationError("Database URL is required. Use .database() to set it.");
    }
    if (!this.config.apiKeySalt) {
      throw new ConfigurationError("API key salt is required. Use .salt() to set it.");
    }

    return Captchapi.create(this.config as CaptchaConfig, {
      skipVersionCheck: this.skipVersionCheck,
    });
  }
}

// =============================================================================
// Lifecycle Callbacks
// =============================================================================

export interface LifecycleCallbacks {
  /** Called when the API is initialized */
  onInitialized?: (api: Captchapi) => void;
  /** Called when the API is closed */
  onClosed?: () => void;
  /** Called when a session is created */
  onSessionCreated?: (sessionId: string) => void;
  /** Called when a session is validated */
  onSessionValidated?: (sessionId: string, valid: boolean) => void;
  /** Called when a session is deleted */
  onSessionDeleted?: (sessionId: string) => void;
  /** Called on any error */
  onError?: (error: Error) => void;
}

// =============================================================================
// Main API Wrapper
// =============================================================================

export interface CaptchapiOptions {
  /** Skip version compatibility check */
  skipVersionCheck?: boolean;
  /** Lifecycle callbacks */
  callbacks?: LifecycleCallbacks;
}

/**
 * High-level wrapper around @captchapi/core with additional convenience features
 *
 * @example
 * ```typescript
 * // Using the builder
 * const api = await CaptchaApiBuilder.create()
 *   .database('./captcha.db')
 *   .salt('your-secret-salt')
 *   .build();
 *
 * // Or using the factory
 * const api = await Captchapi.create({
 *   databaseUrl: 'sqlite:./captcha.db',
 *   apiKeySalt: 'your-secret-salt',
 * });
 *
 * // Create a session
 * const session = await api.createSession({ difficulty: 5 });
 * console.log('Session ID:', session.sessionId);
 *
 * // Validate user input
 * const result = await api.validate(session.sessionId, userAnswer);
 * if (result.valid) {
 *   console.log('CAPTCHA solved!');
 * }
 *
 * // Clean up
 * await api.close();
 * ```
 */
export class Captchapi {
  private static instance: Captchapi | null = null;
  private static instanceConfig: string | null = null;

  private readonly core: InstanceType<typeof import("@captchapi/core").CaptchaApi>;
  private readonly callbacks: LifecycleCallbacks;
  private closed = false;

  private constructor(
    core: InstanceType<typeof import("@captchapi/core").CaptchaApi>,
    callbacks: LifecycleCallbacks = {}
  ) {
    this.core = core;
    this.callbacks = callbacks;
  }

  /**
   * Create a new Captchapi instance
   *
   * @param config Configuration options
   * @param options Additional options
   */
  static async create(
    config: CaptchaConfig,
    options: CaptchapiOptions = {}
  ): Promise<Captchapi> {
    debug("Creating CaptchaApi instance with config:", {
      databaseUrl: config.databaseUrl,
      defaultSessionTtlSeconds: config.defaultSessionTtlSeconds,
      maxSessionTtlSeconds: config.maxSessionTtlSeconds,
      maxValidationAttempts: config.maxValidationAttempts,
      runMigrations: config.runMigrations,
    });

    // Version compatibility check
    if (!options.skipVersionCheck) {
      const coreVersion = getCoreVersion();
      if (!isCompatibleCoreVersion()) {
        throw new VersionCompatibilityError(
          coreVersion,
          `>=${MIN_CORE_VERSION} <${MAX_CORE_VERSION}`
        );
      }
      debug("Core version check passed:", coreVersion);
    }

    // Load native bindings
    let CaptchaApi: typeof import("@captchapi/core").CaptchaApi;
    try {
      const core = await import("@captchapi/core");
      CaptchaApi = core.CaptchaApi;
    } catch (error) {
      const env = getEnvironment();
      const helpMessage = `
Failed to load native bindings for ${env.platform}-${env.arch}.

Possible solutions:
1. Ensure you're using a supported platform (Windows, macOS, Linux)
2. Try reinstalling: npm rebuild @captchapi/core
3. Check if prebuilt binaries are available for your platform
4. For Alpine Linux, ensure you're using the musl variant

Debug info:
- Node.js: ${env.nodeVersion}
- Platform: ${env.platform}
- Architecture: ${env.arch}
`;
      debugError("Failed to load native bindings", error);
      throw new NativeBindingError(helpMessage, error as Error);
    }

    // Create core instance
    try {
      const coreInstance = await CaptchaApi.create(config);
      const instance = new Captchapi(coreInstance, options.callbacks);

      debug("CaptchaApi instance created successfully");
      options.callbacks?.onInitialized?.(instance);

      return instance;
    } catch (error) {
      debugError("Failed to create CaptchaApi instance", error);
      options.callbacks?.onError?.(error as Error);
      throw error;
    }
  }

  /**
   * Get or create a singleton instance
   *
   * @param config Configuration (only used on first call)
   * @param options Additional options
   */
  static async getInstance(
    config: CaptchaConfig,
    options: CaptchapiOptions = {}
  ): Promise<Captchapi> {
    const configKey = JSON.stringify(config);

    if (Captchapi.instance && Captchapi.instanceConfig === configKey) {
      debug("Returning existing singleton instance");
      return Captchapi.instance;
    }

    if (Captchapi.instance) {
      debug("Config changed, closing existing instance");
      await Captchapi.instance.close();
    }

    Captchapi.instance = await Captchapi.create(config, options);
    Captchapi.instanceConfig = configKey;
    return Captchapi.instance;
  }

  /**
   * Clear the singleton instance
   */
  static async clearInstance(): Promise<void> {
    if (Captchapi.instance) {
      await Captchapi.instance.close();
      Captchapi.instance = null;
      Captchapi.instanceConfig = null;
    }
  }

  // ---------------------------------------------------------------------------
  // Session Methods
  // ---------------------------------------------------------------------------

  /**
   * Create a new CAPTCHA session
   */
  async createSession(options?: CreateSessionOptions): Promise<SessionResult> {
    this.ensureNotClosed();
    debug("Creating session with options:", options);

    try {
      const result = await this.core.createSession(options);
      debug("Session created:", result.sessionId);
      this.callbacks.onSessionCreated?.(result.sessionId);
      return result;
    } catch (error) {
      debugError("Failed to create session", error);
      this.callbacks.onError?.(error as Error);
      throw new SessionError("Failed to create session", undefined, error as Error);
    }
  }

  /**
   * Validate a CAPTCHA solution
   */
  async validate(sessionId: string, solution: string): Promise<ValidationResult> {
    this.ensureNotClosed();
    debug("Validating session:", sessionId);

    try {
      const result = await this.core.validate(sessionId, solution);
      debug("Validation result:", { sessionId, valid: result.valid });
      this.callbacks.onSessionValidated?.(sessionId, result.valid);
      return result;
    } catch (error) {
      debugError("Failed to validate session", error);
      this.callbacks.onError?.(error as Error);
      throw new SessionError("Failed to validate session", sessionId, error as Error);
    }
  }

  /**
   * Get the CAPTCHA image for a session
   */
  async getImage(sessionId: string): Promise<Buffer> {
    this.ensureNotClosed();
    debug("Getting image for session:", sessionId);

    try {
      return await this.core.getImage(sessionId);
    } catch (error) {
      debugError("Failed to get image", error);
      this.callbacks.onError?.(error as Error);
      throw new SessionError("Failed to get image", sessionId, error as Error);
    }
  }

  /**
   * Get session information
   */
  async getSession(sessionId: string): Promise<SessionInfo> {
    this.ensureNotClosed();
    debug("Getting session info:", sessionId);

    try {
      return await this.core.getSession(sessionId);
    } catch (error) {
      debugError("Failed to get session", error);
      this.callbacks.onError?.(error as Error);
      throw new SessionError("Failed to get session info", sessionId, error as Error);
    }
  }

  /**
   * Delete a session
   */
  async deleteSession(sessionId: string): Promise<boolean> {
    this.ensureNotClosed();
    debug("Deleting session:", sessionId);

    try {
      const deleted = await this.core.deleteSession(sessionId);
      if (deleted) {
        this.callbacks.onSessionDeleted?.(sessionId);
      }
      return deleted;
    } catch (error) {
      debugError("Failed to delete session", error);
      this.callbacks.onError?.(error as Error);
      throw new SessionError("Failed to delete session", sessionId, error as Error);
    }
  }

  /**
   * Clean up expired sessions
   */
  async cleanupExpired(): Promise<number> {
    this.ensureNotClosed();
    debug("Cleaning up expired sessions");

    try {
      const count = await this.core.cleanupExpired();
      debug("Cleaned up sessions:", count);
      return count;
    } catch (error) {
      debugError("Failed to cleanup expired sessions", error);
      this.callbacks.onError?.(error as Error);
      throw error;
    }
  }

  // ---------------------------------------------------------------------------
  // Stateless Generation
  // ---------------------------------------------------------------------------

  /**
   * Generate a CAPTCHA without storing it
   */
  generate(options?: GenerateOptions): GenerateResult {
    this.ensureNotClosed();
    debug("Generating stateless CAPTCHA with options:", options);
    return this.core.generate(options);
  }

  // ---------------------------------------------------------------------------
  // API Key Methods
  // ---------------------------------------------------------------------------

  /**
   * Create a new API key
   */
  async createApiKey(description?: string): Promise<CreateApiKeyResult> {
    this.ensureNotClosed();
    debug("Creating API key with description:", description);

    try {
      return await this.core.createApiKey(description);
    } catch (error) {
      debugError("Failed to create API key", error);
      this.callbacks.onError?.(error as Error);
      throw error;
    }
  }

  /**
   * Validate an API key
   */
  async validateApiKey(apiKey: string): Promise<boolean> {
    this.ensureNotClosed();
    debug("Validating API key");

    try {
      return await this.core.validateApiKey(apiKey);
    } catch (error) {
      debugError("Failed to validate API key", error);
      this.callbacks.onError?.(error as Error);
      throw error;
    }
  }

  /**
   * Get API key information by hash
   */
  async getApiKey(keyHash: string): Promise<ApiKeyInfo | null> {
    this.ensureNotClosed();
    debug("Getting API key by hash");

    try {
      return await this.core.getApiKey(keyHash);
    } catch (error) {
      debugError("Failed to get API key", error);
      this.callbacks.onError?.(error as Error);
      throw error;
    }
  }

  /**
   * List all API keys
   */
  async listApiKeys(): Promise<ApiKeyInfo[]> {
    this.ensureNotClosed();
    debug("Listing API keys");

    try {
      return await this.core.listApiKeys();
    } catch (error) {
      debugError("Failed to list API keys", error);
      this.callbacks.onError?.(error as Error);
      throw error;
    }
  }

  /**
   * Update an API key
   */
  async updateApiKey(
    keyHash: string,
    isActive?: boolean,
    description?: string
  ): Promise<boolean> {
    this.ensureNotClosed();
    debug("Updating API key:", keyHash);

    try {
      return await this.core.updateApiKey(keyHash, isActive, description);
    } catch (error) {
      debugError("Failed to update API key", error);
      this.callbacks.onError?.(error as Error);
      throw error;
    }
  }

  /**
   * Delete an API key
   */
  async deleteApiKey(keyHash: string): Promise<boolean> {
    this.ensureNotClosed();
    debug("Deleting API key:", keyHash);

    try {
      return await this.core.deleteApiKey(keyHash);
    } catch (error) {
      debugError("Failed to delete API key", error);
      this.callbacks.onError?.(error as Error);
      throw error;
    }
  }

  // ---------------------------------------------------------------------------
  // Lifecycle
  // ---------------------------------------------------------------------------

  /**
   * Check if the instance has been closed
   */
  get isClosed(): boolean {
    return this.closed;
  }

  /**
   * Close the database connection
   */
  async close(): Promise<void> {
    if (this.closed) {
      debug("Instance already closed");
      return;
    }

    debug("Closing CaptchaApi instance");
    this.closed = true;

    try {
      await this.core.close();
      this.callbacks.onClosed?.();
      debug("CaptchaApi instance closed");
    } catch (error) {
      debugError("Error closing instance", error);
      this.callbacks.onError?.(error as Error);
      throw error;
    }
  }

  private ensureNotClosed(): void {
    if (this.closed) {
      throw new CaptchapiError(
        "CaptchaApi instance has been closed",
        "INSTANCE_CLOSED"
      );
    }
  }
}

// =============================================================================
// Quick Start Helpers
// =============================================================================

/**
 * Quick start: Create a CaptchaApi with minimal configuration
 *
 * Uses a file-based SQLite database and generates a random salt.
 * Good for development and testing, but use CaptchaApiBuilder for production.
 *
 * @param dbPath Path to the SQLite database file (default: "./captcha.db")
 * @param salt API key salt (default: random)
 *
 * @example
 * ```typescript
 * const api = await quickStart();
 * const session = await api.createSession();
 * ```
 */
export async function quickStart(
  dbPath = "./captcha.db",
  salt?: string
): Promise<Captchapi> {
  const actualSalt =
    salt ?? `captchapi-dev-${Date.now()}-${Math.random().toString(36)}`;

  if (!salt) {
    console.warn(
      "[captchapi] Warning: Using auto-generated salt. For production, provide a stable salt."
    );
  }

  return Captchapi.create({
    databaseUrl: `sqlite:${dbPath}`,
    apiKeySalt: actualSalt,
  });
}

/**
 * Quick start: Create a CaptchaApi with in-memory database
 *
 * Good for testing. Data is lost when the process exits.
 *
 * @example
 * ```typescript
 * const api = await inMemory();
 * const session = await api.createSession();
 * ```
 */
export async function inMemory(salt = "test-salt"): Promise<Captchapi> {
  return Captchapi.create({
    databaseUrl: "sqlite::memory:",
    apiKeySalt: salt,
  });
}

// =============================================================================
// Default Export
// =============================================================================

export default Captchapi;

// Also export the builder as a named export
export { CaptchaApiBuilder as Builder };
