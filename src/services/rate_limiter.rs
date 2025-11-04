use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

/// Simple in-memory rate limiter using sliding window
#[derive(Clone)]
pub struct RateLimiter {
    /// Map of IP addresses to their request timestamps
    requests: Arc<Mutex<HashMap<IpAddr, Vec<u64>>>>,
    /// Maximum requests allowed in the window
    max_requests: u32,
    /// Window size in seconds
    window_seconds: u64,
}

impl RateLimiter {
    pub fn new(max_requests: u32, window_seconds: u64) -> Self {
        Self {
            requests: Arc::new(Mutex::new(HashMap::new())),
            max_requests,
            window_seconds,
        }
    }

    /// Check if a request from the given IP should be allowed
    ///
    /// Returns true if the request is within rate limits, false otherwise
    pub async fn check_rate_limit(&self, ip: IpAddr) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut requests = self.requests.lock().await;

        // Get or create request history for this IP
        let ip_requests = requests.entry(ip).or_insert_with(Vec::new);

        // Remove timestamps outside the current window
        let window_start = now.saturating_sub(self.window_seconds);
        ip_requests.retain(|&timestamp| timestamp >= window_start);

        // Check if we're within the rate limit
        if ip_requests.len() < self.max_requests as usize {
            // Add this request timestamp
            ip_requests.push(now);
            true
        } else {
            // Rate limit exceeded
            false
        }
    }

    /// Cleanup old entries to prevent unbounded memory growth
    ///
    /// Should be called periodically (e.g., every few minutes)
    pub async fn cleanup(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();

        let mut requests = self.requests.lock().await;

        // Remove IPs with no recent requests
        let window_start = now.saturating_sub(self.window_seconds);
        requests.retain(|_, timestamps| {
            timestamps.retain(|&ts| ts >= window_start);
            !timestamps.is_empty()
        });
    }

    /// Get statistics about rate limiter state (for monitoring)
    pub async fn stats(&self) -> RateLimiterStats {
        let requests = self.requests.lock().await;
        RateLimiterStats {
            tracked_ips: requests.len(),
            max_requests: self.max_requests,
            window_seconds: self.window_seconds,
        }
    }
}

#[derive(Debug)]
pub struct RateLimiterStats {
    pub tracked_ips: usize,
    pub max_requests: u32,
    pub window_seconds: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[tokio::test]
    async fn test_rate_limit_allows_within_limit() {
        let limiter = RateLimiter::new(3, 60);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        // First 3 requests should be allowed
        assert!(limiter.check_rate_limit(ip).await);
        assert!(limiter.check_rate_limit(ip).await);
        assert!(limiter.check_rate_limit(ip).await);
    }

    #[tokio::test]
    async fn test_rate_limit_blocks_over_limit() {
        let limiter = RateLimiter::new(3, 60);
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        // First 3 requests should be allowed
        assert!(limiter.check_rate_limit(ip).await);
        assert!(limiter.check_rate_limit(ip).await);
        assert!(limiter.check_rate_limit(ip).await);

        // 4th request should be blocked
        assert!(!limiter.check_rate_limit(ip).await);
    }

    #[tokio::test]
    async fn test_rate_limit_per_ip() {
        let limiter = RateLimiter::new(2, 60);
        let ip1 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));

        // Each IP should have its own limit
        assert!(limiter.check_rate_limit(ip1).await);
        assert!(limiter.check_rate_limit(ip1).await);
        assert!(!limiter.check_rate_limit(ip1).await); // Blocked

        // ip2 should still be allowed
        assert!(limiter.check_rate_limit(ip2).await);
        assert!(limiter.check_rate_limit(ip2).await);
        assert!(!limiter.check_rate_limit(ip2).await); // Blocked
    }

    #[tokio::test]
    async fn test_rate_limit_with_ipv6() {
        let limiter = RateLimiter::new(2, 60);
        let ip = IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1));

        assert!(limiter.check_rate_limit(ip).await);
        assert!(limiter.check_rate_limit(ip).await);
        assert!(!limiter.check_rate_limit(ip).await);
    }

    #[tokio::test]
    async fn test_cleanup_removes_old_entries() {
        let limiter = RateLimiter::new(10, 1); // 1 second window
        let ip = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));

        // Make a request
        assert!(limiter.check_rate_limit(ip).await);

        // Check stats before cleanup
        let stats_before = limiter.stats().await;
        assert_eq!(stats_before.tracked_ips, 1);

        // Wait for window to expire
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Cleanup should remove the expired entry
        limiter.cleanup().await;

        let stats_after = limiter.stats().await;
        assert_eq!(stats_after.tracked_ips, 0);
    }

    #[tokio::test]
    async fn test_stats() {
        let limiter = RateLimiter::new(100, 60);
        let ip1 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1));
        let ip2 = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));

        limiter.check_rate_limit(ip1).await;
        limiter.check_rate_limit(ip2).await;

        let stats = limiter.stats().await;
        assert_eq!(stats.tracked_ips, 2);
        assert_eq!(stats.max_requests, 100);
        assert_eq!(stats.window_seconds, 60);
    }
}
