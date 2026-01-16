//! Rate limiter configuration for tower_governor.
//!
//! This module provides the configuration struct for rate limiting with support
//! for both direct IP extraction and reverse proxy scenarios.

/// Configuration for the rate limiter
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimiterConfig {
    /// Requests allowed per second (sustained rate)
    pub requests_per_second: u64,
    /// Maximum burst size
    pub burst_size: u32,
    /// Whether to use reverse proxy mode (reads X-Forwarded-For headers)
    pub reverse_proxy: bool,
}

impl RateLimiterConfig {
    /// Create a new rate limiter configuration
    pub fn new(requests_per_second: u64, burst_size: u32, reverse_proxy: bool) -> Self {
        Self {
            requests_per_second,
            burst_size,
            reverse_proxy,
        }
    }

    /// Create a configuration for direct connections (no reverse proxy)
    #[allow(dead_code)]
    pub fn direct(requests_per_second: u64, burst_size: u32) -> Self {
        Self::new(requests_per_second, burst_size, false)
    }

    /// Create a configuration for reverse proxy mode
    #[allow(dead_code)]
    pub fn for_reverse_proxy(requests_per_second: u64, burst_size: u32) -> Self {
        Self::new(requests_per_second, burst_size, true)
    }
}

impl Default for RateLimiterConfig {
    fn default() -> Self {
        Self {
            requests_per_second: 2,
            burst_size: 10,
            reverse_proxy: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rate_limiter_config_new() {
        let config = RateLimiterConfig::new(5, 20, true);
        assert_eq!(config.requests_per_second, 5);
        assert_eq!(config.burst_size, 20);
        assert!(config.reverse_proxy);
    }

    #[test]
    fn test_rate_limiter_config_direct() {
        let config = RateLimiterConfig::direct(10, 50);
        assert_eq!(config.requests_per_second, 10);
        assert_eq!(config.burst_size, 50);
        assert!(!config.reverse_proxy);
    }

    #[test]
    fn test_rate_limiter_config_for_reverse_proxy() {
        let config = RateLimiterConfig::for_reverse_proxy(3, 15);
        assert_eq!(config.requests_per_second, 3);
        assert_eq!(config.burst_size, 15);
        assert!(config.reverse_proxy);
    }

    #[test]
    fn test_rate_limiter_config_default() {
        let config = RateLimiterConfig::default();
        assert_eq!(config.requests_per_second, 2);
        assert_eq!(config.burst_size, 10);
        assert!(!config.reverse_proxy);
    }

    #[test]
    fn test_rate_limiter_config_clone() {
        let config = RateLimiterConfig::new(10, 50, true);
        let cloned = config.clone();
        assert_eq!(config, cloned);
    }

    #[test]
    fn test_rate_limiter_config_debug() {
        let config = RateLimiterConfig::default();
        let debug_str = format!("{:?}", config);
        assert!(debug_str.contains("requests_per_second"));
        assert!(debug_str.contains("burst_size"));
        assert!(debug_str.contains("reverse_proxy"));
    }

    #[test]
    fn test_rate_limiter_config_equality() {
        let config1 = RateLimiterConfig::new(5, 20, true);
        let config2 = RateLimiterConfig::new(5, 20, true);
        let config3 = RateLimiterConfig::new(5, 20, false);

        assert_eq!(config1, config2);
        assert_ne!(config1, config3);
    }

    #[test]
    fn test_rate_limiter_config_different_values() {
        let config1 = RateLimiterConfig::new(1, 5, false);
        let config2 = RateLimiterConfig::new(2, 5, false);
        let config3 = RateLimiterConfig::new(1, 10, false);

        assert_ne!(config1, config2); // Different requests_per_second
        assert_ne!(config1, config3); // Different burst_size
    }
}
