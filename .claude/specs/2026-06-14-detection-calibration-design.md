# Per-Creator Calibrated Highlight Detection — Design Spec (v1.5.0)

**Status:** Approved design, pre-implementation. Author: Claude + Slug, 2026-06-14.
**Goal:** Make clip detection robust across creators and content by scoring each moment as a *departure from that creator's own baseline*, replacing the fixed score threshold that starves loud/low-variance streams. Ship a predictable two-handle sensitivity control and a stable, honest 0–100 score.

---

## 1. Background & root cause

A 99-minute VOD (`2793041100`) with healthy content (1,278 speech segments) produced **1 clip from 35 candidates**. Log: `final 1 clips from 35 candidates (scores: 52%)`. A comparable April VOD produced 11.

**Root cause:** raw scores are not comparable across VODs. This VOD's audio ran loud throughout (`avg_rms 0.461` vs a normal `0.293`; adaptive audio threshold `0.692` vs `0.440`), which compressed the entire candidate-score distribution below curation's **fixed** floor. In `clip_selector.rs`, `min_total_score = (0.50 * scale).clamp(0.50, 0.55)` hard-rejects everything below ~0.50, `diversify_final_selection` only picks survivors, and there is **no minimum-yield floor**. The single survivor (52%) is the loud intro music — the one thing loud enough to clear an inflated bar.

**Secondary bug:** the `.clamp(0.50, 0.55)` means **Medium and High sensitivity use the identical 0.50 score floor** (High's intended drop to ~0.41 is clamped away). "Set to High" only raises the clip-count ceiling, which does nothing when candidates are rejected first. The sensitivity control is partly a placebo today.

## 2. Decisions (locked)

- **Scoring is relative to each stream's own baseline** ("exciting *for this creator*"), not an absolute meter.
- **Hybrid selection (cap + floor):** up to N of the best, but nothing below a real quality floor.
- **Calibration is per-creator**, not per-VOD — durable across a creator's streams.
- **Cold-start = self-bootstrapping:** a creator's baseline seeds from their *own first VOD* (the within-VOD baseline is always available); no shipped style corpus. Slug's VOD library is used only to tune scale-free sanity constants and as the regression test set.
- **Display score:** a single **frozen global 0–100 map** so "70" means the same hype on every stream — discriminative across the full range (no vanity 75–99 compression).
- **Sensitivity UI:** **two decoupled handles** — clip-count ceiling + minimum-score floor — surfaced as Low/Med/High presets plus an advanced override.
- **One release (v1.5.0)**, built and validated in internal layers (calibration core → personalization → UI).

## 3. Non-goals (v1.5.0)

- Per-game event detection via HUD CV/OCR (kill-feeds, "VICTORY") — a future signal; Powder.gg died maintaining 40+ per-game models.
- Deep-learning personalization (T-AIN) — we implement the cheap z-score/EWMA equivalent.
- Any telemetry or global corpus collection — everything is local and self-bootstrapping.

## 4. Competitive context (why this is a moat)

Confirmed across OpusClip, Eklipse, StreamLadder, Medal, Spikes, AutoClip, Powder, Clipbot, Sizzle, Vizard, Quso, Submagic, Klap, Munch, Crayo, Gling, Ssemble:

- **No tool calibrates the detection baseline to an individual creator's loudness/energy.** Eklipse openly tells users to "lower your music bed" — they punt our exact bug to the user's mic mixer.
- **The only per-account learner (Ssemble) adapts on published-performance, not on the audio baseline and not on edit decisions.** So our two moat axes — (a) per-creator baseline calibration, (b) learning from the creator's keep/trim/trash decisions — are genuinely unoccupied.
- **Adopt:** Vizard's two-handle controls (count ceiling + score floor) = our hybrid model; Sizzle's chat×gameplay co-occurrence signal; the field-standard Hook/Flow/Value/Trend score decomposition (meets user expectations).
- **Avoid:** Quso's vanity 75–99 score band (kills discriminative power); per-game model sprawl (Powder).

## 5. Architecture

The redesign inserts a **calibration layer** between the existing raw-signal extraction (audio RMS, chat velocity via `twitch_chat_replay.rs`, transcript emotion/keywords via whisper) and selection, and replaces the fixed-threshold rejection with a two-gate selector. Seven focused units:

### 5.1 `SignalBaseline` (within-VOD, two-timescale) — pure function
For each signal stream walked along the VOD timeline, maintain a **slow EWMA** (long half-life ≈ "this stream's normal") and a **fast EWMA** (short half-life ≈ "now"), plus a cautious EWMA variance.
- Calibrated value: `z(t) = (fast(t) − slow(t)) / sqrt(var_slow(t) + ε²)`, with a **variance floor ε** so a flat/dead signal can't amplify trivial blips into fake spikes.
- Inputs: a time series (audio RMS, chat msgs/sec, emotion intensity). Output: a z-score series. No external dependency — always available, even on a creator's very first VOD.
- Starting constants (tuned in implementation): slow half-life ≈ 90 s, fast half-life ≈ 5 s, ε set per-signal from the sanity-constant pass.

### 5.2 `CreatorBaseline` (cross-VOD, persisted) — the per-creator memory
Per `channel_id`, per signal kind: running `ewma_mean`, `ewma_var`, `n_vods`, `updated_at`.
- **Purpose:** stabilize the absolute floor across a creator's streams (one weird VOD doesn't skew), and encode "this creator runs hot/cold."
- **Cold-start:** on a creator's first VOD, seed from that VOD's aggregate stats (from `SignalBaseline`). No shipped corpus.
- **Empirical-Bayes shrinkage:** the effective baseline used for a VOD is `w·creator + (1−w)·this_vod`, `w = n_vods/(n_vods + k)` (k ≈ 2). Early VODs lean on within-VOD stats; later VODs lean on the accumulated creator baseline.
- **Update:** after each analysis, EWMA the creator baseline toward this VOD's aggregate stats.
- Interface: `get_or_init(channel_id) -> CreatorBaseline`, `update(channel_id, vod_stats)`.

### 5.3 `MomentScorer` (per-candidate composite)
Keep the existing dimensional structure in `clip_selector.rs` (hook, emotional_spike, payoff, replay, etc.) **but feed it calibrated z-values instead of raw signal levels.** Add:
- **Chat×play co-occurrence bonus:** when a chat-velocity z-spike aligns in time with an audio/gameplay z-spike (Sizzle's idea), boost the score.
- **Multi-signal agreement guard:** a candidate must have ≥2 of {audio z, chat z, emotion z} firing to qualify (with graceful relaxation when a signal is unavailable — see §8). Kills single-channel artifacts.
- Output: composite raw score `s` (in calibrated/z units).

### 5.4 `DisplayCalibrator` (frozen global `s → 0–100`)
A single fixed monotonic map, identical for every creator/VOD: `display = 100 · logistic(w·(s − m))` (or a two-anchor linear map). Constants `(w, m)` are tuned **once** against Slug's VODs as reference and shipped. Must be **discriminative across the full range** (no compression to 75–99). This is the number shown to the user and the number the score-floor acts on, which is what makes "≥70" meaningful and stable across streams.

### 5.5 `Selector` (two gates) — replaces `evaluate_rejection` + fixed clamp
- **Gate A — absolute quality floor (no-noise guarantee):** a candidate must clear the preset's `min_score` (on the 0–100 display scale) AND the scale-free sanity gates: minimum raw audio energy (rules out true silence), minimum-duration sustain, multi-signal agreement, and hysteresis (a higher score to *open* a clip, a lower one to *close* it, so a borderline moment doesn't fragment). A dead stream clears nothing → few/zero clips, by design.
- **Gate B — relative cap (no-starvation guarantee):** among qualified candidates, take the top `max_clips` by score, then apply the existing diversity / cooldown / dedup / min-gap logic. Never manufactures a clip below the floor; never starves a content-rich stream.

### 5.6 `SensitivityConfig` (two-handle presets) — fixes the placebo
Low/Med/High each map to a `(max_clips, min_score)` pair on the 0–100 scale (`max_clips` also scales with VOD duration). Illustrative starting values (tuned during implementation against the validation VODs):
| Preset | min_score (floor) | max_clips (×duration target) |
|---|---|---|
| Low (strict) | ~70 | 0.6× |
| Medium | ~55 | 0.8× |
| High (generous) | ~45 | 1.4× |
- **Advanced override:** expose `max_clips` and `min_score` directly for power users. Named framing (Loose/Balanced/Strict) à la Gling.
- Because `min_score` now genuinely differs per preset (no clamp floor), Medium ≠ High — the placebo is fixed.

### 5.7 `EditFeedback` (per-creator learning) — the second moat axis, minimal in v1.5.0
Track which detected clips the creator **keeps, edits/trims, or deletes**, and nudge that creator's scoring toward what they actually clip. v1.5.0 scope is deliberately minimal: a per-creator, per-signal **weight/offset** learned from kept-vs-deleted ratios (e.g., if a creator consistently keeps chat-driven clips and trashes pure-audio ones, up-weight chat for them). Starts neutral; adjusts as the creator acts. Not a model — a handful of persisted weights. *This is the most novel/risky layer; build it last and gate it behind validation (§9). If it balloons, it degrades gracefully to "weights all neutral" = pure calibration.*

## 6. Data flow

1. VOD analyzed → raw signals extracted (existing).
2. `CreatorBaseline.get_or_init(channel_id)` (seed from this VOD if first).
3. `SignalBaseline` → per-signal z-score series (within-VOD, shrinkage-blended with creator baseline per §5.2).
4. Candidate moments generated (existing fusion) using calibrated z.
5. `MomentScorer` → composite `s` per candidate + multi-signal agreement + chat×play bonus + per-creator `EditFeedback` weights.
6. `DisplayCalibrator` → 0–100 display score per candidate.
7. `Selector` Gate A (floor + sanity) → Gate B (top `max_clips` + diversity) → final clips.
8. `CreatorBaseline.update(channel_id, vod_stats)`.
9. On user keep/trim/delete → `EditFeedback.update(channel_id, …)`.

## 7. Data model (new)

- **`creator_baselines`**: `channel_id`, `signal_kind`, `ewma_mean`, `ewma_var`, `n_vods`, `updated_at`. (PK: channel_id + signal_kind.)
- **`creator_signal_weights`** (EditFeedback): `channel_id`, `signal_kind`, `weight`, `updated_at`.
- **`highlights`**: store the calibrated `display_score` (0–100) and the per-signal z contributions (for transparency / the score badge), alongside existing columns.
- **`detection_sensitivity`** setting extended to carry the optional advanced `(max_clips, min_score)` override.
- All via a `db.rs` migration; existing highlights keep their old `virality_score` (new analyses use the new score).

## 8. Error handling & edge cases

- **Chat replay missing/failing** (note: an existing `[community-clips]` 400 bug — `parsing time "…2026-06-12T03:20:44 00:00"` — is now **FIXED 2026-06-14** by URL-encoding the started_at/ended_at/cursor query values in `twitch::fetch_community_clips`; chat is a key signal): degrade gracefully — drop the chat signal, require ≥2 of the *available* signals, fall back to audio+transcript agreement. Detection still works, just without the chat axis.
- **Brand-new creator / first VOD:** self-bootstraps from the within-VOD baseline (§5.2). No error, no empty-corpus failure.
- **Genuinely dead/silent VOD:** Gate A passes nothing → return few/zero clips AND surface a clear **"low-activity VOD"** message in the UI (the lesson from this bug: never show a mysterious empty result).
- **Flat/near-constant signal:** the variance floor ε prevents divide-by-zero and blip amplification.

## 9. Testing & validation

**Unit tests (Rust):**
- `SignalBaseline`: synthetic loud-baseline signal → real spikes still detected; flat signal → no spikes (ε floor holds).
- `Selector` two-gate: the bug reproduced as a test — 35 candidates with compressed scores → top-K rescued above the floor; all-dead candidates → zero pass Gate A.
- `DisplayCalibrator`: monotonic, full 0–100 range, stable.
- `SensitivityConfig`: Low/Med/High yield **different** `min_score` (placebo fixed).
- `CreatorBaseline`: first VOD seeds; EWMA updates; shrinkage weight `w = n/(n+k)` behaves.

**Real-VOD acceptance gates (the decisive test — run the new pipeline on Slug's existing transcripts/audio):**
- Broken loud VOD `2793041100`: **1 → ~6+ clips**, and the clips are real gameplay moments, not the intro.
- April working VODs (e.g. `2736880526` = 11, `2745878262` = 15): stay in a healthy band — **no collapse and no ballooning** (e.g., within ±30% of prior counts, quality preserved).
- A regression harness compares clip counts + selected timestamps before/after per VOD.

## 10. Files touched (estimate)

- `src-tauri/src/clip_selector.rs` — biggest change: replace `evaluate_rejection` (fixed clamp) + `diversify_final_selection` with the two-gate `Selector`; feed calibrated z.
- **New** `src-tauri/src/signal_calibration.rs` — `SignalBaseline` (two-timescale EWMA), `DisplayCalibrator`, scorer calibration helpers.
- **New** `src-tauri/src/creator_baseline.rs` — `CreatorBaseline` + `EditFeedback` stores.
- `src-tauri/src/db.rs` — new tables + migration.
- `src-tauri/src/commands/vod.rs` — wire baseline load/update into the analysis pipeline; "low-activity VOD" messaging.
- `src/pages/Settings.tsx` — two-handle controls (presets + advanced override).
- `src/pages/Clips.tsx` / Editor — display the calibrated 0–100 score badge; low-activity message; wire keep/delete into `EditFeedback`.

## 11. Build sequencing (one v1.5.0, internal layers with validation gates)

1. **Calibration core** — `SignalBaseline` + `DisplayCalibrator` + two-gate `Selector` + `SensitivityConfig`, using within-VOD baselines only. **Gate:** broken VOD → ~6+, April VODs healthy. This alone fixes the bug.
2. **Per-creator memory** — `CreatorBaseline` + shrinkage cold-start + persistence. **Gate:** stable across a creator's multiple VODs.
3. **Personalization** — `EditFeedback` weights. **Gate:** kept-clip alignment improves without destabilizing counts; degrades safely to neutral.
4. **UI** — two-handle Settings controls + score badge + low-activity messaging.

## 12. Open risks

- **Cold-start UX:** a brand-new creator's first 1–2 VODs are "calibrating"; consider a subtle UI note rather than pretending precision.
- **Constant tuning:** several constants (half-lives, ε, the `s→100` map, preset pairs) are tuned against Slug's VODs — they're gaming-streamer-derived, so revisit once other creators' data exists.
- **EditFeedback** is the most speculative layer; kept minimal and gated so it can't regress the core fix.
- **Regression surface:** this rewrites the scoring/curation core — the real-VOD harness (§9) is the guardrail and must pass before release.
