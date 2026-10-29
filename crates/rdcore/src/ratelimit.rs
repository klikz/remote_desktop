//! IP bo'yicha noto'g'ri urinishlarni cheklash (brute-force'dan himoya).

use std::{
    collections::HashMap,
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

struct Entry {
    failures: u32,
    window_start: Instant,
    blocked_until: Option<Instant>,
}

pub struct RateLimiter {
    inner: Mutex<HashMap<IpAddr, Entry>>,
    max_failures: u32,
    window: Duration,
    block: Duration,
}

impl RateLimiter {
    /// `window` ichida `max_failures` marta xato qilgan IP `block` muddatga bloklanadi.
    pub fn new(max_failures: u32, window: Duration, block: Duration) -> Self {
        RateLimiter {
            inner: Mutex::new(HashMap::new()),
            max_failures,
            window,
            block,
        }
    }

    /// Standart sozlama: 10 daqiqada 5 xato -> 15 daqiqa blok.
    pub fn standard() -> Self {
        Self::new(5, Duration::from_secs(600), Duration::from_secs(900))
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, Entry>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_blocked(&self, ip: IpAddr) -> bool {
        self.is_blocked_at(ip, Instant::now())
    }

    pub fn record_failure(&self, ip: IpAddr) {
        self.record_failure_at(ip, Instant::now())
    }

    pub fn record_success(&self, ip: IpAddr) {
        self.map().remove(&ip);
    }

    pub fn is_blocked_at(&self, ip: IpAddr, now: Instant) -> bool {
        let mut map = self.map();
        match map.get(&ip).and_then(|e| e.blocked_until) {
            Some(until) if now < until => true,
            Some(_) => {
                map.remove(&ip);
                false
            }
            None => false,
        }
    }

    pub fn record_failure_at(&self, ip: IpAddr, now: Instant) {
        let mut map = self.map();
        let e = map.entry(ip).or_insert(Entry {
            failures: 0,
            window_start: now,
            blocked_until: None,
        });
        if now.duration_since(e.window_start) > self.window {
            e.failures = 0;
            e.window_start = now;
        }
        e.failures += 1;
        if e.failures >= self.max_failures {
            e.blocked_until = Some(now + self.block);
        }
        // Xotira o'smasligi uchun eski yozuvlarni tozalash.
        if map.len() > 10_000 {
            let window = self.window;
            map.retain(|_, e| {
                e.blocked_until.map_or(false, |u| now < u)
                    || now.duration_since(e.window_start) <= window
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(n: u8) -> IpAddr {
        IpAddr::from([10, 0, 0, n])
    }

    fn limiter() -> RateLimiter {
        RateLimiter::new(3, Duration::from_secs(60), Duration::from_secs(300))
    }

    #[test]
    fn blocks_after_max_failures() {
        let l = limiter();
        let t0 = Instant::now();
        assert!(!l.is_blocked_at(ip(1), t0));
        l.record_failure_at(ip(1), t0);
        l.record_failure_at(ip(1), t0);
        assert!(!l.is_blocked_at(ip(1), t0));
        l.record_failure_at(ip(1), t0);
        assert!(l.is_blocked_at(ip(1), t0));
    }

    #[test]
    fn other_ips_unaffected() {
        let l = limiter();
        let t0 = Instant::now();
        for _ in 0..3 {
            l.record_failure_at(ip(1), t0);
        }
        assert!(l.is_blocked_at(ip(1), t0));
        assert!(!l.is_blocked_at(ip(2), t0));
    }

    #[test]
    fn block_expires() {
        let l = limiter();
        let t0 = Instant::now();
        for _ in 0..3 {
            l.record_failure_at(ip(1), t0);
        }
        assert!(l.is_blocked_at(ip(1), t0 + Duration::from_secs(299)));
        assert!(!l.is_blocked_at(ip(1), t0 + Duration::from_secs(301)));
        // blok tugagach hisob nolga qaytadi
        l.record_failure_at(ip(1), t0 + Duration::from_secs(302));
        assert!(!l.is_blocked_at(ip(1), t0 + Duration::from_secs(302)));
    }

    #[test]
    fn failures_outside_window_do_not_accumulate() {
        let l = limiter();
        let t0 = Instant::now();
        l.record_failure_at(ip(1), t0);
        l.record_failure_at(ip(1), t0);
        // oyna tugagach hisob qaytadan boshlanadi
        l.record_failure_at(ip(1), t0 + Duration::from_secs(61));
        assert!(!l.is_blocked_at(ip(1), t0 + Duration::from_secs(61)));
    }

    #[test]
    fn success_resets() {
        let l = limiter();
        let t0 = Instant::now();
        l.record_failure_at(ip(1), t0);
        l.record_failure_at(ip(1), t0);
        l.record_success(ip(1));
        l.record_failure_at(ip(1), t0);
        assert!(!l.is_blocked_at(ip(1), t0));
    }
}
