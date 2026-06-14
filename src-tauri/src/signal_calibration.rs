//! Per-stream signal calibration: score a moment by how far it departs from
//! the stream's OWN rolling baseline, so a loud stream and a chill stream are
//! scored on the same footing. See .claude/specs/2026-06-14-detection-calibration-design.md.

/// An online exponentially-weighted mean + variance (Welford-style EWMA).
#[derive(Clone, Debug)]
pub struct EwmaStat {
    pub mean: f64,
    pub var: f64,
    alpha: f64,
    initialized: bool,
}

impl EwmaStat {
    /// `alpha` in (0,1]: higher = faster adaptation (shorter memory).
    pub fn new(alpha: f64) -> Self {
        Self { mean: 0.0, var: 0.0, alpha: alpha.clamp(1e-4, 1.0), initialized: false }
    }

    pub fn update(&mut self, x: f64) {
        if !self.initialized {
            self.mean = x;
            self.var = 0.0;
            self.initialized = true;
            return;
        }
        let delta = x - self.mean;
        self.mean += self.alpha * delta;
        // EWMA of squared deviation (variance) with the same rate.
        self.var = (1.0 - self.alpha) * (self.var + self.alpha * delta * delta);
    }
}

/// Two-timescale baseline: a SLOW EWMA ("this stream's normal") and a FAST
/// EWMA ("right now"). The calibrated value is how far `fast` sits above
/// `slow`, in units of the slow baseline's standard deviation.
#[derive(Clone, Debug)]
pub struct RollingBaseline {
    slow: EwmaStat,
    fast: EwmaStat,
    var_floor: f64,
}

impl RollingBaseline {
    /// `dt` = sample spacing (s); half-lives in seconds. `var_floor` prevents a
    /// flat/dead signal from amplifying trivial blips into huge z-scores.
    pub fn new(dt: f64, slow_halflife: f64, fast_halflife: f64, var_floor: f64) -> Self {
        Self {
            slow: EwmaStat::new(alpha_from_halflife(dt, slow_halflife)),
            fast: EwmaStat::new(alpha_from_halflife(dt, fast_halflife)),
            var_floor: var_floor.max(1e-9),
        }
    }

    /// Feed the next raw sample; returns its calibrated z-score (departure from
    /// the stream's normal). Non-negative spikes are the interesting case.
    pub fn push(&mut self, x: f64) -> f64 {
        self.slow.update(x);
        self.fast.update(x);
        let std = (self.slow.var + self.var_floor).sqrt();
        (self.fast.mean - self.slow.mean) / std
    }
}

/// Convert a half-life (seconds) to an EWMA alpha given sample spacing `dt`.
pub fn alpha_from_halflife(dt: f64, halflife: f64) -> f64 {
    if halflife <= 0.0 { return 1.0; }
    1.0 - (-(dt / halflife) * std::f64::consts::LN_2).exp()
}

/// Frozen, global score → 0–100 map. Identical for every creator/VOD so "70"
/// means the same hype everywhere. Logistic squash; tuned ONCE (validation) and
/// shipped. Must stay discriminative across the full range (no vanity band).
#[derive(Clone, Debug)]
pub struct DisplayCalibrator {
    /// Composite score that maps to 50.
    pub midpoint: f64,
    /// Slope; larger = steeper transition around the midpoint.
    pub slope: f64,
}

impl Default for DisplayCalibrator {
    fn default() -> Self {
        // Starting constants; final tuning happens against the real VODs when the
        // v1.5.0 build is run live (the synthetic test only exercises the mechanism).
        Self { midpoint: 0.55, slope: 6.0 }
    }
}

impl DisplayCalibrator {
    pub fn to_display(&self, s: f64) -> f64 {
        let z = self.slope * (s - self.midpoint);
        100.0 / (1.0 + (-z).exp())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spike_on_loud_baseline_still_registers() {
        // Constant-loud stream (0.8) for 60 samples, then a brief spike to 1.5.
        let mut b = RollingBaseline::new(1.0, 90.0, 5.0, 1e-4);
        let mut last_baseline_z = 0.0;
        for _ in 0..60 { last_baseline_z = b.push(0.8); }
        let spike_z = b.push(1.5);
        // At steady loud baseline, z is near zero; the spike is clearly positive
        // and well above the baseline reading.
        assert!(last_baseline_z.abs() < 0.5, "baseline z should be ~0, got {last_baseline_z}");
        assert!(spike_z > last_baseline_z + 1.0, "spike z {spike_z} should exceed baseline {last_baseline_z}");
    }

    #[test]
    fn flat_dead_signal_does_not_amplify() {
        // A near-silent flat stream with a 1% blip must NOT produce a huge z
        // (the var_floor guards against divide-by-tiny-variance).
        let mut b = RollingBaseline::new(1.0, 90.0, 5.0, 1e-2);
        for _ in 0..60 { b.push(0.01); }
        let blip_z = b.push(0.011);
        assert!(blip_z < 1.0, "flat-signal blip z should stay small, got {blip_z}");
    }

    #[test]
    fn display_map_is_monotonic_full_range_and_centered() {
        let d = DisplayCalibrator::default();
        // Monotonic increasing.
        let mut prev = -1.0;
        for i in -30..=30 {
            let s = i as f64 / 10.0; // -3.0 ..= 3.0
            let v = d.to_display(s);
            assert!(v >= prev, "not monotonic at s={s}: {v} < {prev}");
            assert!((0.0..=100.0).contains(&v), "out of range at s={s}: {v}");
            prev = v;
        }
        // Centered: the midpoint maps to 50.
        assert!((d.to_display(DisplayCalibrator::default().midpoint) - 50.0).abs() < 0.5);
        // Discriminative (NOT compressed to a vanity band).
        assert!(d.to_display(1.0) - d.to_display(0.0) > 10.0);
    }
}
