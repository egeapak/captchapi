//! Centralized validation logic for session parameters, solutions, and API keys.
//!
//! This module is the single source of truth for all input validation,
//! shared between the HTTP server and NAPI bindings.

use rand::distr::Alphanumeric;
use rand::RngExt;

// Session parameter boundaries
pub const DIFFICULTY_MIN: i64 = 1;
pub const DIFFICULTY_MAX: i64 = 10;
pub const LENGTH_MIN: i64 = 1;
pub const LENGTH_MAX: i64 = 20;
pub const WIDTH_MIN: i64 = 50;
pub const WIDTH_MAX: i64 = 1000;
pub const HEIGHT_MIN: i64 = 30;
pub const HEIGHT_MAX: i64 = 500;
pub const COMPRESSION_MIN: i64 = 1;
pub const COMPRESSION_MAX: i64 = 100;
pub const SOLUTION_MAX_LEN: usize = 100;

// Default values
/// Difficulty applied when a request does not name one.
///
/// This has been 5, then 8, and now 5 again, and the round trip is the point:
/// the number that matters is the solve rate, and the renderer changed
/// underneath it.
///
/// It went to 8 because difficulty 5 was solvable — three vision models were
/// given 27 challenges each across lengths 4-6 and took 13 of 27 at level 5,
/// Sonnet alone 8 of 9. Nearly every deformation scales with this value (see
/// `services::captcha::generator::Deformations::for_difficulty`), and the old
/// linear ramp left level 5 in the band where they barely applied.
///
/// Two later changes moved that band down. `INTENSITY_CURVE` made the ramp
/// concave, so level 5 now carries the intensity the linear ramp gave level 7,
/// and the blur moved after the composite, where it smears clustered letters
/// into each other instead of softening their edges.
///
/// **What level 5 actually measures, pooled over every set solved against the
/// current renderer — 84 attempts on 30 distinct images, vision-only and
/// tool-equipped, Opus 5 and Sonnet 5:**
///
/// ```text
/// vision-only          4/60  =  6.7%   95% CI [2.6%, 15.9%]
/// with image tools     3/24  = 12.5%   95% CI [4.3%, 31.0%]
/// pooled               7/84  =  8.3%   95% CI [4.1%, 16.2%]
/// ```
///
/// With `MAX_VALIDATION_ATTEMPTS` at 3 that is an 12-41% chance of defeating one
/// session, and the honest summary is that **level 5 is a real CAPTCHA against
/// frontier models but not a wall.** Two individual sets came back 0/18 and it
/// would have been wrong to call that a floor: a zero on 18 attempts has a 95%
/// upper bound near 18% on its own, and a third set of fresh images drew 4/24.
/// Quote the pooled interval, not a lucky cell.
///
/// The reason 5 is nonetheless defensible as the default is what the tooled arms
/// found. Image processing **stopped helping**: paired per image and per model,
/// tools won 3 and lost 4, p = 1.0, at 5-6x the token cost (255-319k against
/// ~50k). On the old renderer tooling was decisive at this level, 3/3 against
/// 2/3. `gradient` and the post-composite blur are why — hue splitting was the
/// tooled attack's main weapon, and a letter no longer has one hue while its
/// boundary with the next letter is now a hue gradient rather than a step.
///
/// If this needs to be lower, raise the difficulty rather than adding a
/// deformation, and measure the level you pick: current 6/7/8 are unmeasured,
/// and the old renderer's 1/24 at level 8 does not transfer to a renderer whose
/// intensity ramp has changed shape.
///
/// `blur` does not scale with this value; it is held flat at every level above
/// 1. See `FLAT_BLUR`.
pub const DEFAULT_DIFFICULTY: i64 = 5;
pub const DEFAULT_LENGTH: i64 = 5;
pub const DEFAULT_WIDTH: i64 = 220;
pub const DEFAULT_HEIGHT: i64 = 120;
pub const DEFAULT_DARK_MODE: bool = false;
pub const DEFAULT_COMPRESSION: i64 = 40;

// API key constants
pub const API_KEY_LENGTH: usize = 32;
pub const MAX_DESCRIPTION_LENGTH: usize = 255;

/// Validated and defaulted session creation parameters.
#[derive(Debug, Clone)]
pub struct ValidatedSessionParams {
    pub length: i64,
    pub difficulty: i64,
    pub width: i64,
    pub height: i64,
    pub dark_mode: bool,
    pub compression: i64,
    pub expires_in: u64,
}

/// Validate and apply defaults for session creation parameters.
///
/// Returns a `ValidatedSessionParams` with all values resolved and range-checked,
/// or an error message describing the first validation failure.
#[allow(clippy::too_many_arguments)]
pub fn validate_session_params(
    length: Option<i64>,
    difficulty: Option<i64>,
    width: Option<i64>,
    height: Option<i64>,
    dark_mode: Option<bool>,
    compression: Option<i64>,
    expires_in_seconds: Option<u64>,
    default_ttl: u64,
    max_ttl: u64,
) -> Result<ValidatedSessionParams, String> {
    let expires_in = expires_in_seconds.unwrap_or(default_ttl);
    if expires_in > max_ttl {
        return Err(format!(
            "expires_in_seconds cannot exceed {} seconds",
            max_ttl
        ));
    }

    let difficulty = difficulty.unwrap_or(DEFAULT_DIFFICULTY);
    if !(DIFFICULTY_MIN..=DIFFICULTY_MAX).contains(&difficulty) {
        return Err(format!(
            "difficulty must be between {} and {}",
            DIFFICULTY_MIN, DIFFICULTY_MAX
        ));
    }

    let length = length.unwrap_or(DEFAULT_LENGTH);
    if !(LENGTH_MIN..=LENGTH_MAX).contains(&length) {
        return Err(format!(
            "length must be between {} and {} characters",
            LENGTH_MIN, LENGTH_MAX
        ));
    }

    let width = width.unwrap_or(DEFAULT_WIDTH);
    if !(WIDTH_MIN..=WIDTH_MAX).contains(&width) {
        return Err(format!(
            "width must be between {} and {} pixels",
            WIDTH_MIN, WIDTH_MAX
        ));
    }

    let height = height.unwrap_or(DEFAULT_HEIGHT);
    if !(HEIGHT_MIN..=HEIGHT_MAX).contains(&height) {
        return Err(format!(
            "height must be between {} and {} pixels",
            HEIGHT_MIN, HEIGHT_MAX
        ));
    }

    let dark_mode = dark_mode.unwrap_or(DEFAULT_DARK_MODE);

    let compression = compression.unwrap_or(DEFAULT_COMPRESSION);
    if !(COMPRESSION_MIN..=COMPRESSION_MAX).contains(&compression) {
        return Err(format!(
            "compression must be between {} and {}",
            COMPRESSION_MIN, COMPRESSION_MAX
        ));
    }

    Ok(ValidatedSessionParams {
        length,
        difficulty,
        width,
        height,
        dark_mode,
        compression,
        expires_in,
    })
}

/// Validate a CAPTCHA solution input (length limit to prevent abuse).
pub fn validate_solution(solution: &str) -> Result<(), String> {
    if solution.len() > SOLUTION_MAX_LEN {
        return Err(format!(
            "solution must not exceed {} characters",
            SOLUTION_MAX_LEN
        ));
    }
    Ok(())
}

/// Generate a random API key (32-char alphanumeric string).
pub fn generate_api_key() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(API_KEY_LENGTH)
        .map(char::from)
        .collect()
}

/// Validate an API key description.
///
/// Returns `Ok(())` if valid, or `Err` with a descriptive error message if invalid.
pub fn validate_api_key_description(description: &Option<String>) -> Result<(), String> {
    if let Some(desc) = description {
        // Check if empty/whitespace only
        if desc.trim().is_empty() {
            return Err("Description cannot be empty or whitespace only".to_string());
        }

        // Check length
        if desc.len() > MAX_DESCRIPTION_LENGTH {
            return Err(format!(
                "Description exceeds maximum length of {} characters",
                MAX_DESCRIPTION_LENGTH
            ));
        }

        // Check for control characters (except newlines and tabs which are acceptable)
        if desc
            .chars()
            .any(|c| c.is_control() && c != '\n' && c != '\t' && c != '\r')
        {
            return Err("Description contains invalid control characters".to_string());
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- validate_session_params ---

    #[test]
    fn test_defaults_applied() {
        let params =
            validate_session_params(None, None, None, None, None, None, None, 300, 3600).unwrap();
        assert_eq!(params.length, DEFAULT_LENGTH);
        assert_eq!(params.difficulty, DEFAULT_DIFFICULTY);
        assert_eq!(params.width, DEFAULT_WIDTH);
        assert_eq!(params.height, DEFAULT_HEIGHT);
        assert_eq!(params.dark_mode, DEFAULT_DARK_MODE);
        assert_eq!(params.compression, DEFAULT_COMPRESSION);
        assert_eq!(params.expires_in, 300);
    }

    #[test]
    fn test_all_params_provided() {
        let params = validate_session_params(
            Some(10),
            Some(8),
            Some(400),
            Some(200),
            Some(true),
            Some(80),
            Some(600),
            300,
            3600,
        )
        .unwrap();
        assert_eq!(params.length, 10);
        assert_eq!(params.difficulty, 8);
        assert_eq!(params.width, 400);
        assert_eq!(params.height, 200);
        assert!(params.dark_mode);
        assert_eq!(params.compression, 80);
        assert_eq!(params.expires_in, 600);
    }

    #[test]
    fn test_expires_in_exceeds_max() {
        let result =
            validate_session_params(None, None, None, None, None, None, Some(3601), 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("cannot exceed 3600"));
    }

    #[test]
    fn test_expires_in_at_max() {
        let params =
            validate_session_params(None, None, None, None, None, None, Some(3600), 300, 3600)
                .unwrap();
        assert_eq!(params.expires_in, 3600);
    }

    #[test]
    fn test_difficulty_below_min() {
        let result =
            validate_session_params(None, Some(0), None, None, None, None, None, 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("difficulty"));
    }

    #[test]
    fn test_difficulty_above_max() {
        let result =
            validate_session_params(None, Some(11), None, None, None, None, None, 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("difficulty"));
    }

    #[test]
    fn test_difficulty_at_boundaries() {
        assert!(
            validate_session_params(None, Some(1), None, None, None, None, None, 300, 3600).is_ok()
        );
        assert!(
            validate_session_params(None, Some(10), None, None, None, None, None, 300, 3600)
                .is_ok()
        );
    }

    #[test]
    fn test_length_below_min() {
        let result =
            validate_session_params(Some(0), None, None, None, None, None, None, 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("length"));
    }

    #[test]
    fn test_length_above_max() {
        let result =
            validate_session_params(Some(21), None, None, None, None, None, None, 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("length"));
    }

    #[test]
    fn test_length_at_boundaries() {
        assert!(
            validate_session_params(Some(1), None, None, None, None, None, None, 300, 3600).is_ok()
        );
        assert!(
            validate_session_params(Some(20), None, None, None, None, None, None, 300, 3600)
                .is_ok()
        );
    }

    #[test]
    fn test_width_below_min() {
        let result =
            validate_session_params(None, None, Some(49), None, None, None, None, 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("width"));
    }

    #[test]
    fn test_width_above_max() {
        let result =
            validate_session_params(None, None, Some(1001), None, None, None, None, 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("width"));
    }

    #[test]
    fn test_width_at_boundaries() {
        assert!(
            validate_session_params(None, None, Some(50), None, None, None, None, 300, 3600)
                .is_ok()
        );
        assert!(
            validate_session_params(None, None, Some(1000), None, None, None, None, 300, 3600)
                .is_ok()
        );
    }

    #[test]
    fn test_height_below_min() {
        let result =
            validate_session_params(None, None, None, Some(29), None, None, None, 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("height"));
    }

    #[test]
    fn test_height_above_max() {
        let result =
            validate_session_params(None, None, None, Some(501), None, None, None, 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("height"));
    }

    #[test]
    fn test_height_at_boundaries() {
        assert!(
            validate_session_params(None, None, None, Some(30), None, None, None, 300, 3600)
                .is_ok()
        );
        assert!(
            validate_session_params(None, None, None, Some(500), None, None, None, 300, 3600)
                .is_ok()
        );
    }

    #[test]
    fn test_compression_below_min() {
        let result =
            validate_session_params(None, None, None, None, None, Some(0), None, 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("compression"));
    }

    #[test]
    fn test_compression_above_max() {
        let result =
            validate_session_params(None, None, None, None, None, Some(101), None, 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("compression"));
    }

    #[test]
    fn test_compression_at_boundaries() {
        assert!(
            validate_session_params(None, None, None, None, None, Some(1), None, 300, 3600).is_ok()
        );
        assert!(
            validate_session_params(None, None, None, None, None, Some(100), None, 300, 3600)
                .is_ok()
        );
    }

    // --- validate_solution ---

    #[test]
    fn test_solution_at_limit() {
        let solution = "a".repeat(SOLUTION_MAX_LEN);
        assert!(validate_solution(&solution).is_ok());
    }

    #[test]
    fn test_solution_above_limit() {
        let solution = "a".repeat(SOLUTION_MAX_LEN + 1);
        let result = validate_solution(&solution);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("100 characters"));
    }

    #[test]
    fn test_solution_empty() {
        assert!(validate_solution("").is_ok());
    }

    // --- generate_api_key ---

    #[test]
    fn test_api_key_length() {
        let key = generate_api_key();
        assert_eq!(key.len(), API_KEY_LENGTH);
    }

    #[test]
    fn test_api_key_alphanumeric() {
        let key = generate_api_key();
        assert!(key.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_api_key_uniqueness() {
        let key1 = generate_api_key();
        let key2 = generate_api_key();
        assert_ne!(key1, key2);
    }

    // --- validate_api_key_description ---

    #[test]
    fn test_description_none_ok() {
        assert!(validate_api_key_description(&None).is_ok());
    }

    #[test]
    fn test_description_valid() {
        assert!(validate_api_key_description(&Some("My API Key".to_string())).is_ok());
    }

    #[test]
    fn test_description_empty_rejected() {
        assert!(validate_api_key_description(&Some("".to_string())).is_err());
    }

    #[test]
    fn test_description_too_long_rejected() {
        let long = "a".repeat(256);
        assert!(validate_api_key_description(&Some(long)).is_err());
    }

    #[test]
    fn test_description_whitespace_only_rejected() {
        assert!(validate_api_key_description(&Some("   ".to_string())).is_err());
        assert!(validate_api_key_description(&Some("\t\n".to_string())).is_err());
    }

    #[test]
    fn test_description_control_chars_rejected() {
        assert!(validate_api_key_description(&Some("test\x00key".to_string())).is_err());
        assert!(validate_api_key_description(&Some("test\x07key".to_string())).is_err());
    }

    #[test]
    fn test_description_allows_newlines_and_tabs() {
        assert!(validate_api_key_description(&Some("line1\nline2".to_string())).is_ok());
        assert!(validate_api_key_description(&Some("col1\tcol2".to_string())).is_ok());
    }

    #[test]
    fn test_description_at_max_length_ok() {
        let desc = "a".repeat(MAX_DESCRIPTION_LENGTH);
        assert!(validate_api_key_description(&Some(desc)).is_ok());
    }

    #[test]
    fn test_negative_difficulty_rejected() {
        let result =
            validate_session_params(None, Some(-1), None, None, None, None, None, 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("difficulty"));
    }

    #[test]
    fn test_negative_length_rejected() {
        let result =
            validate_session_params(Some(-1), None, None, None, None, None, None, 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("length"));
    }

    #[test]
    fn test_negative_width_rejected() {
        let result =
            validate_session_params(None, None, Some(-1), None, None, None, None, 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("width"));
    }

    #[test]
    fn test_negative_height_rejected() {
        let result =
            validate_session_params(None, None, None, Some(-1), None, None, None, 300, 3600);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("height"));
    }

    #[test]
    fn test_expires_in_zero_ok() {
        let params =
            validate_session_params(None, None, None, None, None, None, Some(0), 300, 3600)
                .unwrap();
        assert_eq!(params.expires_in, 0);
    }

    #[test]
    fn test_solution_normal_valid() {
        assert!(validate_solution("abc123").is_ok());
        assert!(validate_solution("X7kP2m").is_ok());
    }
}
