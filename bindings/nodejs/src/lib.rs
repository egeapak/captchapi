//! Node.js native bindings for CaptchAPI
//!
//! This crate provides NAPI-RS bindings for using CaptchAPI as a native
//! Node.js module. It exposes a high-level `CaptchaApi` class that can be
//! used directly from JavaScript/TypeScript.
//!
//! # Usage
//!
//! ```typescript
//! import { CaptchaApi } from '@captchapi/core';
//!
//! const api = await CaptchaApi.create({
//!   databaseUrl: 'sqlite:./captcha.db',
//!   apiKeySalt: 'your-secret-salt'
//! });
//!
//! // Create a CAPTCHA session
//! const session = await api.createSession({
//!   difficulty: 5,
//!   width: 220,
//!   height: 120
//! });
//!
//! // Validate the user's answer
//! const result = await api.validate(session.sessionId, userAnswer);
//! ```

mod captcha_api;
mod error;
mod types;

pub use captcha_api::CaptchaApi;
