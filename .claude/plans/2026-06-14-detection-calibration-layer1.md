# Detection Calibration — Layer 1 (Calibration Core) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Run from `C:\Users\cereb\Desktop\Claude projects\clipviral`. Rust verification: `cd src-tauri && cargo test --lib`. Bump version (`powershell -file bump-version.ps1 <ver>`) before any commit per CLAUDE.md.

**Goal:** Fix the loud-stream clip-starvation bug by calibrating scores within each VOD and replacing the fixed-threshold curation with a two-gate selector (absolute quality floor + relative top-K cap) and Low/Med/High presets that actually differ.

**Architecture:** A new pure module `signal_calibration.rs` provides a two-timescale EWMA baseline (z-score = departure from the stream's own normal) and a frozen logistic `score → 0–100` display map. In `clip_selector.rs`, the audio-derived scoring dimensions become baseline-relative, `CurationConfig` gains a real per-sensitivity `min_display_score` (removing the clamp that makes Medium==High), and the fixed-0.50 rejection + `retain` is replaced by a two-gate selector: Gate A = absolute quality gates (hook/emotion/dead-air/vague) + the sensitivity display-score floor; Gate B = rank survivors by score and take the top `max_clips`. Validated by re-running selection on real VOD signals.

**Tech Stack:** Rust, `cargo test --lib`, the existing `clip_selector` pipeline (`select_clips` → fuse → score → reject → diversify).

**Spec:** `.claude/specs/2026-06-14-detection-calibration-design.md`. This plan implements that spec's §5.1, §5.4, §5.5, §5.6 (the within-VOD layer). Per-creator memory (§5.2), edit-feedback (§5.7), and the Settings UI are Layers 2–4, planned after this lands.

---

## File Structure

- **Create `src-tauri/src/signal_calibration.rs`** — `EwmaStat`, `RollingBaseline` (two-timescale z-score), `DisplayCalibrator` (logistic). Pure functions, no I/O. Owns all calibration math.
- **Modify `src-tauri/src/lib.rs`** — add `mod signal_calibration;` to the module list (near line 31, with the other `mod` declarations).
- **Modify `src-tauri/src/clip_selector.rs`** — `CurationConfig` (two-handle + fix clamp), the audio dimension analyzers (baseline-relative), the rejection/selection stages (two-gate), and store the display score on `ClipCandidate`.
- Tests live in inline `#[cfg(test)] mod` blocks in each file (matches the existing pattern — `clip_selector.rs` already has a `mod tests`).

---

### Task 1: `signal_calibration.rs` — two-timescale EWMA baseline

**Files:**
- Create: `src-tauri/src/signal_calibration.rs`
- Modify: `src-tauri/src/lib.rs` (add `mod signal_calibration;`)

- [ ] **Step 1: Create the module with the failing test**

Create `src-tauri/src/signal_calibration.rs`:

```rust
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
}
```

- [ ] **Step 2: Register the module and run the failing test**

Add to `src-tauri/src/lib.rs` near line 31 (with the other `mod` lines): `mod signal_calibration;`

Run: `cd src-tauri && cargo test --lib signal_calibration`
Expected: compiles and the two tests PASS (this module is self-contained; if a test fails, the math/constants are wrong — fix before moving on).

- [ ] **Step 3: Commit**

```
powershell -file bump-version.ps1 1.5.0
git add src-tauri/src/signal_calibration.rs src-tauri/src/lib.rs package.json src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "feat(detection): add RollingBaseline two-timescale EWMA z-score (Layer 1)"
```

---

### Task 2: `signal_calibration.rs` — frozen logistic display map

**Files:**
- Modify: `src-tauri/src/signal_calibration.rs`

- [ ] **Step 1: Add the failing test** (append to the `tests` module)

```rust
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
        // Discriminative (NOT compressed to a vanity band): a clear gap in s
        // produces a clear gap in display.
        assert!(d.to_display(1.0) - d.to_display(0.0) > 10.0);
    }
```

- [ ] **Step 2: Implement `DisplayCalibrator`** (add above the `tests` module)

```rust
/// Frozen, global score → 0–100 map. Identical for every creator/VOD so "70"
/// means the same hype everywhere. Logistic squash; tuned ONCE (Task 7) and
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
        // Starting constants; refined against the validation VODs in Task 7.
        Self { midpoint: 0.55, slope: 6.0 }
    }
}

impl DisplayCalibrator {
    pub fn to_display(&self, s: f64) -> f64 {
        let z = self.slope * (s - self.midpoint);
        100.0 / (1.0 + (-z).exp())
    }
}
```

- [ ] **Step 3: Run the test**

Run: `cd src-tauri && cargo test --lib signal_calibration`
Expected: all signal_calibration tests PASS.

- [ ] **Step 4: Commit**

```
powershell -file bump-version.ps1 1.5.0
git add src-tauri/src/signal_calibration.rs
git commit -m "feat(detection): add frozen logistic DisplayCalibrator (Layer 1)"
```

---

### Task 3: `CurationConfig` — real two-handle presets (fix the Medium==High placebo)

**Files:**
- Modify: `src-tauri/src/clip_selector.rs` (the `CurationConfig` struct ~line 91 and `for_duration` ~line 120, and add a `min_display_score` field)

**Context:** Today `min_total_score = (0.50 * scale).clamp(0.50, 0.55)` — the clamp forces Medium and High to the same 0.50 floor (High's intended drop is clamped away). We add an explicit per-sensitivity **`min_display_score`** (on the 0–100 display scale) that genuinely differs, and keep `min_total_score` only as a soft signal (no longer a hard cliff — see Task 5).

- [ ] **Step 1: Add the failing test** (in the `clip_selector.rs` `tests` module)

```rust
    #[test]
    fn sensitivity_presets_have_distinct_floors_and_caps() {
        let sel = crate::game_config::SelectorConfig::default();
        let low  = CurationConfig::for_duration(99.0 * 60.0, "low", &sel);
        let med  = CurationConfig::for_duration(99.0 * 60.0, "medium", &sel);
        let high = CurationConfig::for_duration(99.0 * 60.0, "high", &sel);
        // Floors must strictly differ across presets (the placebo bug).
        assert!(low.min_display_score > med.min_display_score, "low floor must exceed medium");
        assert!(med.min_display_score > high.min_display_score, "medium floor must exceed high");
        // Caps already differ; keep that property.
        assert!(low.max_clips < med.max_clips && med.max_clips < high.max_clips);
    }
```

- [ ] **Step 2: Run it to confirm it fails**

Run: `cd src-tauri && cargo test --lib sensitivity_presets_have_distinct_floors`
Expected: FAIL — `min_display_score` field does not exist.

- [ ] **Step 3: Add the field and set it per sensitivity**

In the `CurationConfig` struct (near line 91, after `max_clips`), add:
```rust
    /// Minimum 0–100 display score a clip must reach to qualify (the user-facing
    /// quality floor; genuinely differs per sensitivity — see DisplayCalibrator).
    pub min_display_score: f64,
```

In `for_duration` (near line 148, replacing the clamped `min_total_score` block — keep `min_total_score`/`min_hook`/`min_emotion` for the soft quality gates but stop using min_total_score as a hard cliff), add a real floor that differs per preset:
```rust
        let min_display_score = match sensitivity {
            "low"  => 70.0, // strict: only the clearest moments
            "high" => 45.0, // generous
            _      => 55.0, // medium (default)
        };
```

Add `min_display_score` to the returned `Self { … }` (near line 167). Update the existing `log::info!("CurationConfig: …")` to also print `min_display_score`.

- [ ] **Step 4: Run the test**

Run: `cd src-tauri && cargo test --lib sensitivity_presets_have_distinct_floors`
Expected: PASS.

- [ ] **Step 5: Commit**

```
powershell -file bump-version.ps1 1.5.0
git add src-tauri/src/clip_selector.rs
git commit -m "feat(detection): per-sensitivity display-score floor, fixes Medium==High placebo (Layer 1)"
```

---

### Task 4: Calibrate the audio dimensions to the stream's own baseline

**Files:**
- Modify: `src-tauri/src/clip_selector.rs` — `generate_audio_candidates` and/or the `analyze_hook_strength` / `analyze_emotional_spike` functions (read them first; they currently use `AudioContext` levels relative to `avg_rms`).

**Context:** The audio-derived dimensions currently key off near-absolute RMS levels, which a loud baseline inflates uniformly. Replace the absolute reference with a `RollingBaseline` z-score over the audio RMS envelope, so a "spike" means "loud *for this stream*."

- [ ] **Step 1: Add the failing test** (clip_selector `tests` module)

```rust
    #[test]
    fn audio_spike_score_is_baseline_relative() {
        // Two synthetic RMS envelopes with identical spike SHAPE but different
        // baselines (quiet vs loud). The calibrated spike score must be ~equal.
        let quiet = calibrated_spike_score(&envelope(0.20, 0.60)); // base 0.20, spike 0.60
        let loud  = calibrated_spike_score(&envelope(0.50, 0.90)); // base 0.50, spike 0.90
        assert!((quiet - loud).abs() < 0.15,
            "same-shape spikes should score alike: quiet={quiet} loud={loud}");
    }
```

(Add the two helpers `envelope(base, peak) -> Vec<f64>` — 60 base samples then 5 peak samples — and `calibrated_spike_score(env: &[f64]) -> f64` — feed the envelope through `RollingBaseline`, take the max z over the peak window, squash to 0..1 via `(z / 4.0).clamp(0.0, 1.0)` — inside the `tests` module.)

- [ ] **Step 2: Run it to confirm it fails**

Run: `cd src-tauri && cargo test --lib audio_spike_score_is_baseline_relative`
Expected: FAIL — `calibrated_spike_score` not yet wired into scoring (the test helper proves the property; this task threads it into the real analyzers).

- [ ] **Step 3: Thread the baseline z into the audio analyzers**

Read `generate_audio_candidates`, `analyze_hook_strength`, `analyze_emotional_spike`, and the `AudioContext` definition. Replace their absolute-RMS comparisons with a `RollingBaseline` z over the RMS envelope (slow half-life 90s, fast 5s, var_floor tuned in Task 7), squashed to 0..1. Keep the function signatures; change the internals so `emotional_spike`/`hook_strength` reflect departure-from-baseline. The test helper from Step 1 is the reference implementation to mirror.

- [ ] **Step 4: Run the test + the full suite**

Run: `cd src-tauri && cargo test --lib`
Expected: the new test PASSES and the existing 479 tests still pass (some scoring-sensitive tests may need threshold updates — adjust the *test expectations*, not the math, where the new calibrated values are correct).

- [ ] **Step 5: Commit**

```
powershell -file bump-version.ps1 1.5.0
git add src-tauri/src/clip_selector.rs
git commit -m "feat(detection): audio dimensions scored relative to the stream's own baseline (Layer 1)"
```

---

### Task 5: Two-gate selector — replace the fixed-0.50 cliff with floor + top-K

**Files:**
- Modify: `src-tauri/src/clip_selector.rs` — `evaluate_rejection` (~line 615) and the selection stage in `select_clips` (~lines 1159–1181).

**Context:** This is the core bug fix. Today: `evaluate_rejection` hard-rejects `total_score < min_total_score` (0.50), `retain` drops them, and `diversify_final_selection` picks survivors → on a loud VOD, 34 of 35 die. New model: **Gate A** keeps the *absolute quality gates* (hook/emotion/dead-air/vague-single) AND the per-sensitivity `min_display_score` floor; **Gate B** ranks all Gate-A survivors by `total_score` and takes the top `max_clips`. No fixed `total_score` cliff — score is for ranking, not a guillotine.

- [ ] **Step 1: Add the failing test (the bug, reproduced)** (clip_selector `tests` module)

```rust
    #[test]
    fn compressed_scores_are_rescued_by_top_k_not_collapsed() {
        // 35 candidates with compressed scores (loud-stream symptom): all pass
        // the quality gates but cluster at 0.40–0.55. The selector must return a
        // healthy set (top-K), NOT collapse to 1.
        let cfg = {
            let sel = crate::game_config::SelectorConfig::default();
            CurationConfig::for_duration(99.0 * 60.0, "medium", &sel)
        };
        let mut cands: Vec<ClipCandidate> = (0..35).map(|i| {
            let mut c = build_test_candidate(vec![SignalSource::Audio, SignalSource::Chat]);
            c.start_time = (i as f64) * 120.0;
            c.end_time = c.start_time + 25.0;
            c.peak_time = c.start_time + 10.0;
            c.total_score = 0.40 + (i % 4) as f64 * 0.04; // 0.40..0.52
            c
        }).collect();
        let kept = apply_two_gate_selection(&mut cands, 99.0 * 60.0, &cfg);
        assert!(kept.len() >= 6, "expected a healthy set, got {}", kept.len());
        assert!(kept.len() <= cfg.max_clips, "must respect the cap");
    }

    #[test]
    fn dead_air_yields_nothing() {
        // Candidates that fail the absolute quality gates → zero clips (no noise).
        let cfg = {
            let sel = crate::game_config::SelectorConfig::default();
            CurationConfig::for_duration(99.0 * 60.0, "medium", &sel)
        };
        let mut cands: Vec<ClipCandidate> = (0..10).map(|_| {
            let mut c = build_test_candidate(vec![SignalSource::Audio]);
            c.hook_strength = 0.05;     // below min_hook
            c.emotional_spike = 0.05;   // below min_emotion
            c.total_score = 0.05;
            c
        }).collect();
        let kept = apply_two_gate_selection(&mut cands, 99.0 * 60.0, &cfg);
        assert_eq!(kept.len(), 0, "dead-air candidates must yield no clips");
    }
```

- [ ] **Step 2: Run them to confirm they fail**

Run: `cd src-tauri && cargo test --lib two_gate -- --include-ignored; cargo test --lib compressed_scores dead_air`
Expected: FAIL — `apply_two_gate_selection` does not exist.

- [ ] **Step 3: Implement `apply_two_gate_selection` and rewire `select_clips`**

Add a new fn that encapsulates the two gates (reusing the existing quality gates from `evaluate_rejection` minus the `total_score` cliff, and the existing `diversify_final_selection` for Gate B):

```rust
/// Two-gate selection: Gate A = absolute quality gates (hook/emotion/dead-air/
/// vague) + the per-sensitivity display-score floor; Gate B = rank by score and
/// take the top `max_clips` (with the existing diversity/cooldown logic).
fn apply_two_gate_selection(
    candidates: &mut Vec<ClipCandidate>,
    duration: f64,
    cfg: &CurationConfig,
) -> Vec<ClipCandidate> {
    let display = crate::signal_calibration::DisplayCalibrator::default();
    // Gate A: keep candidates that clear the quality gates AND the display floor.
    candidates.retain(|c| {
        passes_quality_gates(c, cfg) && display.to_display(c.total_score) >= cfg.min_display_score
    });
    // Gate B: existing diversity-aware selection already caps at cfg.max_clips
    // and ranks by score — reuse it unchanged.
    diversify_final_selection(candidates, duration, cfg)
}
```

Refactor `evaluate_rejection`'s gate checks into a pure `fn passes_quality_gates(c: &ClipCandidate, cfg: &CurationConfig) -> bool` (hook ≥ min_hook, emotion ≥ min_emotion, not dead-air, not vague-single) — i.e. everything *except* the `total_score < min_total_score` cliff, which is removed. In `select_clips` (Stage 5–7, lines ~1159–1181), replace the `evaluate_rejection` loop + `retain` + `diversify_final_selection` call with a single `let final_clips = apply_two_gate_selection(&mut candidates, duration, &cfg);`. Keep the dedup (`suppress_duplicate_candidates`) and `enforce_minimum_gap` stages before it. Keep the existing `log::info!("Clip selector: final {} clips from {} candidates …")`.

- [ ] **Step 4: Run the new tests + full suite**

Run: `cd src-tauri && cargo test --lib`
Expected: the two new tests PASS; existing suite green (adjust any test that asserted the old fixed-0.50 behavior).

- [ ] **Step 5: Commit**

```
powershell -file bump-version.ps1 1.5.0
git add src-tauri/src/clip_selector.rs
git commit -m "feat(detection): two-gate selector (quality floor + top-K) replaces fixed-0.50 cliff (Layer 1)"
```

---

### Task 6: Persist the 0–100 display score on highlights

**Files:**
- Modify: `src-tauri/src/clip_selector.rs` (compute display score per kept clip), `src-tauri/src/commands/vod.rs` (where highlights are written), `src-tauri/src/db.rs` (store it).

**Context:** The `highlights` table already has `virality_score`. Populate it with the **calibrated 0–100 display score** (via `DisplayCalibrator`) so the UI badge is the stable cross-VOD number. No schema change needed (reuse `virality_score`; document that it is now the 0–100 display score).

- [ ] **Step 1: Add the failing test** (clip_selector `tests` module)

```rust
    #[test]
    fn kept_clips_carry_a_0_100_display_score() {
        let d = crate::signal_calibration::DisplayCalibrator::default();
        let s = 0.7_f64;
        let display = d.to_display(s);
        assert!(display > 50.0 && display <= 100.0, "0.7 raw should map above midpoint, got {display}");
    }
```

- [ ] **Step 2: Run it**

Run: `cd src-tauri && cargo test --lib kept_clips_carry_a_0_100`
Expected: PASS (this asserts the mapping; Step 3 wires it into persistence).

- [ ] **Step 3: Wire the display score into the highlight rows**

Where `select_clips` results are turned into `db::HighlightRow`/insert in `commands/vod.rs`, set the persisted `virality_score` to `DisplayCalibrator::default().to_display(candidate.total_score)` (0–100). Read the highlight-insertion site in `vod.rs` first; keep all other fields unchanged.

- [ ] **Step 4: Run the full suite**

Run: `cd src-tauri && cargo test --lib`
Expected: green.

- [ ] **Step 5: Commit**

```
powershell -file bump-version.ps1 1.5.0
git add src-tauri/src/clip_selector.rs src-tauri/src/commands/vod.rs src-tauri/src/db.rs
git commit -m "feat(detection): persist calibrated 0-100 display score on highlights (Layer 1)"
```

---

### Task 7: Real-VOD validation harness + constant tuning

**Files:**
- Create: `src-tauri/tests/detection_validation.rs` (an integration test, or an inline `#[ignore]` test in clip_selector).

**Context:** The decisive acceptance gate. Re-run `select_clips` on real signals and assert the bug is fixed without regressing good VODs. Slug's transcripts live at `%APPDATA%/clipviral/transcripts/<vod_id>.json`; audio context can be reconstructed from the stored VOD or a captured fixture. Since reconstructing full `AudioContext` in a test is heavy, this task captures **fixture inputs** for two VODs and asserts clip counts.

- [ ] **Step 1: Capture fixtures**

Add a dev-only helper (behind `#[ignore]`) that, given a VOD id, loads its transcript + recomputes audio/chat signals and serializes the `select_clips` inputs to `src-tauri/tests/fixtures/<vod_id>.json`. Capture two fixtures: `2793041100` (the broken loud VOD) and `2736880526` (the April VOD that yielded 11).

- [ ] **Step 2: Write the acceptance test**

```rust
#[test]
fn loud_vod_yields_healthy_set_and_april_vod_does_not_regress() {
    let broken = run_select_clips_on_fixture("2793041100", "medium");
    assert!(broken >= 6, "loud VOD must yield >=6 clips (was 1), got {broken}");
    let april = run_select_clips_on_fixture("2736880526", "medium");
    assert!((6..=18).contains(&april), "April VOD must stay healthy (~11), got {april}");
}
```

- [ ] **Step 3: Run + tune**

Run: `cd src-tauri && cargo test --lib loud_vod_yields_healthy_set`
If the loud VOD still under-yields or the April VOD balloons, tune the constants — `DisplayCalibrator { midpoint, slope }`, the per-sensitivity `min_display_score`, and the RollingBaseline `var_floor` — until both pass. Tune *only* these calibration constants; do not special-case a VOD.

- [ ] **Step 4: Final full suite + commit**

Run: `cd src-tauri && cargo test --lib`
Expected: all green, including the acceptance test.

```
powershell -file bump-version.ps1 1.5.0
git add src-tauri/tests/ src-tauri/src/clip_selector.rs src-tauri/src/signal_calibration.rs
git commit -m "test(detection): real-VOD validation — loud VOD 1->6+, April no regress (Layer 1)"
```

---

## Self-Review

- **Spec coverage (Layer 1 scope):** §5.1 RollingBaseline → Task 1; §5.4 DisplayCalibrator → Task 2; §5.6 two-handle sensitivity → Task 3; relative audio scoring (§5.1/§5.3) → Task 4; §5.5 two-gate selector → Task 5; display score persistence → Task 6; §9 real-VOD acceptance → Task 7. Layer 1's "low-activity VOD" UI message and the per-creator/edit-feedback/Settings-UI items are intentionally Layers 2–4.
- **Placeholder scan:** new-module code is complete; brownfield Tasks 4 & 6 instruct the implementer to read the specific named functions first (the surrounding code is large) and give the exact test + integration point — the test defines correctness. No "TBD".
- **Type consistency:** `DisplayCalibrator::to_display`, `RollingBaseline::push/new`, `CurationConfig.min_display_score`, `apply_two_gate_selection`, `passes_quality_gates` are referenced consistently across tasks.
- **Note:** version is bumped to 1.5.0 once (Task 1) and re-run (idempotent no-op) on later commits per the repo's bump-before-commit rule; one release as decided.
