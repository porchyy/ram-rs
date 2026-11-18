use std::time::Duration;

pub const HIGH: f64 = 90.0;
pub const LOW: f64 = 85.0;
const HOLD: Duration = Duration::from_secs(10);
const COOLDOWN: Duration = Duration::from_secs(120);

#[derive(Default)]
pub struct Policy {
    high_since: Option<Duration>,
    low_since: Option<Duration>,
    last_run: Option<Duration>,
    last_sample: Option<Duration>,
    locked: bool,
}

impl Policy {
    pub fn sample(&mut self, now: Duration, used: Option<f64>, enabled: bool) -> bool {
        // Missing samples and long scheduling gaps cannot prove continuous pressure.
        if self
            .last_sample
            .is_some_and(|last| now.saturating_sub(last) > Duration::from_secs(3))
        {
            self.high_since = None;
            self.low_since = None;
        }
        self.last_sample = Some(now);
        let Some(used) = used.filter(|v| v.is_finite() && (0.0..=100.0).contains(v)) else {
            self.high_since = None;
            self.low_since = None;
            return false;
        };
        if !enabled {
            self.high_since = None;
            self.low_since = None;
            return false;
        }
        if self.locked {
            if self
                .last_run
                .is_some_and(|last| now.saturating_sub(last) >= COOLDOWN)
                && used < LOW
            {
                let start = self.low_since.get_or_insert(now);
                if now.saturating_sub(*start) >= HOLD {
                    self.locked = false;
                    self.low_since = None;
                }
            } else {
                self.low_since = None;
            }
            return false;
        }
        if used >= HIGH {
            let start = self.high_since.get_or_insert(now);
            if now.saturating_sub(*start) >= HOLD {
                self.record_run(now);
                return true;
            }
        } else {
            self.high_since = None;
        }
        false
    }

    pub fn record_run(&mut self, now: Duration) {
        self.last_run = Some(now);
        self.locked = true;
        self.high_since = None;
        self.low_since = None;
    }

    pub fn status(&self) -> &'static str {
        if self.locked {
            "Cooldown / waiting for RAM <85% for 10s after cooldown"
        } else if self.high_since.is_some() {
            "Measuring sustained RAM pressure"
        } else {
            "Ready; waiting for RAM >=90% for 10s"
        }
    }

    pub fn cooling_down(&self) -> bool {
        self.locked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_and_strict_low_boundary_survive_toggling() {
        let mut p = Policy::default();
        for t in 0..10 {
            assert!(!p.sample(Duration::from_secs(t), Some(90.0), true));
        }
        assert!(p.sample(Duration::from_secs(10), Some(90.0), true));
        assert!(p.cooling_down());
        for t in 11..130 {
            assert!(!p.sample(Duration::from_secs(t), Some(80.0), t != 30));
        }
        for t in 130..151 {
            assert!(!p.sample(Duration::from_secs(t), Some(85.0), true));
        }
        for t in 151..162 {
            assert!(!p.sample(Duration::from_secs(t), Some(84.9), true));
        }
        assert!(!p.cooling_down());
        for t in 162..172 {
            assert!(!p.sample(Duration::from_secs(t), Some(90.0), true));
        }
        assert!(p.sample(Duration::from_secs(172), Some(90.0), true));
    }

    #[test]
    fn pressure_cooldown_rearm_and_invalid_samples() {
        let mut p = Policy::default();
        let mut step = |t, v, on| p.sample(Duration::from_secs(t), v, on);
        assert!(!step(0, Some(89.9), true));
        for t in 1..=10 {
            assert!(!step(t, Some(90.0), true));
        }
        assert!(step(11, Some(90.0), true));
        for t in 12..=250 {
            assert!(!step(t, Some(99.0), true));
        }
        for t in 251..=261 {
            assert!(!step(t, Some(84.9), true));
        }
        for t in 262..=271 {
            assert!(!step(t, Some(90.0), true));
        }
        assert!(step(272, Some(90.0), true));

        let mut p = Policy::default();
        for t in 0..10 {
            assert!(!p.sample(Duration::from_secs(t), Some(95.0), true));
        }
        assert!(!p.sample(Duration::from_secs(10), None, true));
        assert!(!p.sample(Duration::from_secs(11), Some(f64::NAN), true));
        for t in 12..22 {
            assert!(!p.sample(Duration::from_secs(t), Some(95.0), true));
        }
        assert!(!p.sample(Duration::from_secs(22), Some(95.0), false));
        assert!(!p.sample(Duration::from_secs(23), Some(95.0), true));
        assert!(!p.sample(Duration::from_secs(40), Some(95.0), true));
        assert!(!p.sample(Duration::from_secs(41), Some(89.0), true));
    }
}
