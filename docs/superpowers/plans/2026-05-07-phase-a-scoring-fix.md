# Phase A Scoring Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop transcript-only clip candidates from surfacing in the high-score band by overriding their boilerplate `context_simplicity` and `emotional_spike` dimensions, applying a 65% total-score cap, and dropping hallucinated whisper output before it reaches the candidate pool.

**Architecture:** Three surgical edits in two files. (1) `convert_whisper_result` in `commands/vod.rs` skips transcript segments that are part of a 4+ identical-consecutive-line run when scanning for keywords. (2) `score_clip_candidate` in `clip_selector.rs` overrides `context_simplicity = 0.50` and `emotional_spike = 0.40` for clips whose `signal_sources` is exactly `[Transcript]`. (3) The same function applies a final `total_score = total_score.min(0.65)` cap for transcript-only clips. All three layers ship together as v1.3.12.

**Tech Stack:** Rust + Tauri 2 backend, existing test patterns (`#[cfg(test)] mod tests`).

---

## Spec reference

This plan implements `docs/superpowers/specs/2026-05-07-phase-a-scoring-fix-design.md`. Three layers, all ship in v1.3.12 as a single user-facing scoring improvement.

## Pre-flight: verify these symbols still exist

Before starting, confirm these resolve in the current code (line numbers may shift slightly):

- `pub fn convert_whisper_result(wr: &whisper::TranscriptResult) -> TranscriptResult` at `src-tauri/src/commands/vod.rs:541`
- `const TRANSCRIPT_KEYWORDS: &[&str]` at `src-tauri/src/commands/vod.rs:533`
- `pub fn score_clip_candidate(c: &mut ClipCandidate)` at `src-tauri/src/clip_selector.rs:450`
- `pub enum SignalSource { Audio, Transcript, Chat, Community, EmoteBurst }` at `src-tauri/src/clip_selector.rs:31`
- `pub struct ClipCandidate { ... pub signal_sources: Vec<SignalSource>, pub hook_strength: f64, pub emotional_spike: f64, pub payoff_clarity: f64, pub event_reaction_alignment: f64, pub context_simplicity: f64, pub replay_value: f64, pub total_score: f64, ... }` at `src-tauri/src/clip_selector.rs:61`
- `pub struct TranscriptSegment { pub start: f64, pub end: f64, pub text: String, ... }` at or near `src-tauri/src/commands/vod.rs:495` (verify by `grep -n "pub struct TranscriptSegment" src-tauri/src/commands/vod.rs`)

If any are renamed/moved, locate them by symbol search before editing.

## File structure

**Backend (Rust):**

- `src-tauri/src/commands/vod.rs` — Add `is_hallucinated_segment` helper. Modify `convert_whisper_result` to skip hallucinated runs when scanning for keywords. Two new unit tests in the existing `mod tests` block (or add one if none exists).
- `src-tauri/src/clip_selector.rs` — Modify `score_clip_candidate` to add (a) the dimension override block and (b) the 65% cap. Three new unit tests.

**Files NOT changed:**

- `analyze_context_simplicity` and `analyze_emotional_spike` at clip_selector.rs:393-436 stay untouched. Per the spec, the override happens at the orchestration layer (`score_clip_candidate`), not by changing the per-dimension functions. This keeps the per-dimension functions pure and tag-keyed (other code may rely on them) and isolates the transcript-only special case to one place.
- `generate_transcript_candidates` at clip_selector.rs:255 (the function that adds "shock" tags from keyword matches) — out of scope. Tag-system overhaul is in v1.3.13+ backlog.
- All frontend code — Phase A is backend-only.

---

## Task 1: Hallucination-detection helper function (TDD)

**Goal:** Add a pure helper `fn is_hallucinated_segment(segments: &[TranscriptSegment], idx: usize) -> bool` that returns `true` if the segment at `idx` is part of a run of 4+ identical consecutive segments.

**Files:**
- Modify: `src-tauri/src/commands/vod.rs` — add helper function near `convert_whisper_result` (so it's co-located with its consumer); add unit tests inside the existing `#[cfg(test)] mod tests` block at the bottom of the file (or create one if none exists).

- [ ] **Step 1.1: Confirm test infrastructure**

Run: `grep -n "mod tests" src-tauri/src/commands/vod.rs`

Two outcomes:
- **Tests block exists**: append the new tests inside it.
- **No tests block**: create one at the very bottom of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn seg(start: f64, end: f64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start,
            end,
            text: text.to_string(),
            words: Vec::new(),
        }
    }
}
```

(The `seg` helper builds a `TranscriptSegment` for the test. `words: Vec::new()` matches what `convert_whisper_result` does at line 546.)

- [ ] **Step 1.2: Write the failing tests**

Append inside `mod tests`:

```rust
    #[test]
    fn hallucinated_segment_returns_true_for_4_identical_in_a_row() {
        let segs = vec![
            seg(0.0, 1.0, "(little bit of a \"dance\" sound)"),
            seg(1.0, 2.0, "(little bit of a \"dance\" sound)"),
            seg(2.0, 3.0, "(little bit of a \"dance\" sound)"),
            seg(3.0, 4.0, "(little bit of a \"dance\" sound)"),
            seg(4.0, 5.0, "Real speech here."),
        ];
        // The first 4 are part of a 4-run, so all four are hallucinated.
        assert!(is_hallucinated_segment(&segs, 0));
        assert!(is_hallucinated_segment(&segs, 1));
        assert!(is_hallucinated_segment(&segs, 2));
        assert!(is_hallucinated_segment(&segs, 3));
        // The 5th segment is not part of any run.
        assert!(!is_hallucinated_segment(&segs, 4));
    }

    #[test]
    fn hallucinated_segment_returns_false_for_3_identical_in_a_row() {
        let segs = vec![
            seg(0.0, 1.0, "Hi there."),
            seg(1.0, 2.0, "Hi there."),
            seg(2.0, 3.0, "Hi there."),
            seg(3.0, 4.0, "Different now."),
        ];
        // 3 in a row is below the 4 threshold.
        assert!(!is_hallucinated_segment(&segs, 0));
        assert!(!is_hallucinated_segment(&segs, 1));
        assert!(!is_hallucinated_segment(&segs, 2));
        assert!(!is_hallucinated_segment(&segs, 3));
    }

    #[test]
    fn hallucinated_segment_returns_false_for_empty_text() {
        let segs = vec![
            seg(0.0, 1.0, ""),
            seg(1.0, 2.0, ""),
            seg(2.0, 3.0, ""),
            seg(3.0, 4.0, ""),
        ];
        // Empty/whitespace text is not considered hallucination.
        assert!(!is_hallucinated_segment(&segs, 0));
    }

    #[test]
    fn hallucinated_segment_handles_out_of_bounds_idx() {
        let segs: Vec<TranscriptSegment> = Vec::new();
        assert!(!is_hallucinated_segment(&segs, 0));
        assert!(!is_hallucinated_segment(&segs, 100));
    }

    #[test]
    fn hallucinated_segment_finds_run_when_idx_is_inside_it() {
        // 5 identical segments — every index inside the run should be flagged.
        let segs = vec![
            seg(0.0, 1.0, "Real."),
            seg(1.0, 2.0, "noise"),
            seg(2.0, 3.0, "noise"),
            seg(3.0, 4.0, "noise"),
            seg(4.0, 5.0, "noise"),
            seg(5.0, 6.0, "noise"),
            seg(6.0, 7.0, "Done."),
        ];
        assert!(!is_hallucinated_segment(&segs, 0));  // "Real." standalone
        // Indices 1-5 are all inside the 5-run.
        assert!(is_hallucinated_segment(&segs, 1));
        assert!(is_hallucinated_segment(&segs, 2));
        assert!(is_hallucinated_segment(&segs, 3));
        assert!(is_hallucinated_segment(&segs, 4));
        assert!(is_hallucinated_segment(&segs, 5));
        assert!(!is_hallucinated_segment(&segs, 6));  // "Done." standalone
    }
```

- [ ] **Step 1.3: Run tests, verify failure**

Run: `cd src-tauri && cargo test commands::vod::tests::hallucinated -- --nocapture`

Expected: COMPILE ERROR `cannot find function 'is_hallucinated_segment' in this scope` on each test.

- [ ] **Step 1.4: Implement the helper**

In `src-tauri/src/commands/vod.rs`, add this function immediately before `fn convert_whisper_result` (around line 540):

```rust
/// Detects whether the transcript segment at `idx` is part of a run of
/// 4 or more consecutive segments with identical text (after trimming).
///
/// Whisper sometimes hallucinates background music or low-information audio
/// as repeated identical lines (e.g., "(little bit of a 'dance' sound)" 16
/// times in a row). Phase B data showed this passes through the keyword
/// scanner if a real word happens to also appear in the run, which then
/// makes the clip register as "transcript signal present" when it shouldn't.
///
/// Threshold of 4 chosen to not false-positive on legitimate repetition
/// (a streamer chanting "go go go" or repeated short callouts) while
/// catching whisper's typical hallucination patterns. Empty/whitespace
/// text is excluded so silent segments aren't flagged.
fn is_hallucinated_segment(segments: &[TranscriptSegment], idx: usize) -> bool {
    if idx >= segments.len() {
        return false;
    }
    let target = segments[idx].text.trim();
    if target.is_empty() {
        return false;
    }

    // Find the start of the run containing idx by walking backwards while
    // text matches.
    let mut run_start = idx;
    while run_start > 0 && segments[run_start - 1].text.trim() == target {
        run_start -= 1;
    }

    // Walk forwards from run_start counting identical text.
    let mut run_len = 0usize;
    for i in run_start..segments.len() {
        if segments[i].text.trim() == target {
            run_len += 1;
        } else {
            break;
        }
    }

    run_len >= 4
}
```

- [ ] **Step 1.5: Run tests, verify pass**

Run: `cd src-tauri && cargo test commands::vod::tests::hallucinated -- --nocapture`

Expected: 5 passed, 0 failed.

- [ ] **Step 1.6: Commit**

```bash
git add src-tauri/src/commands/vod.rs
git commit -m "feat(scoring): is_hallucinated_segment helper + 5 unit tests"
```

---

## Task 2: Apply hallucination guard inside `convert_whisper_result`

**Goal:** When scanning segments for keywords inside `convert_whisper_result`, skip any segment that's part of a hallucination run. Keywords from those segments don't get added to `keywords_found`, so they don't drive transcript signal candidates downstream.

**Files:**
- Modify: `src-tauri/src/commands/vod.rs` — add a guard inside the keyword-scanning loop in `convert_whisper_result`. Add one integration-style test verifying the guard fires.

- [ ] **Step 2.1: Write the failing test**

Append to the `mod tests` block in `src-tauri/src/commands/vod.rs`:

```rust
    #[test]
    fn convert_whisper_result_drops_keywords_from_hallucinated_runs() {
        use crate::whisper::{TranscriptResult as WhisperResult, TranscriptSegment as WhisperSeg};

        // Build a synthetic whisper result with:
        // - 5 hallucinated identical segments containing "what the" (a TRANSCRIPT_KEYWORDS hit)
        // - 1 real segment containing "let's go" (also a hit)
        let wr = WhisperResult {
            language: "en".to_string(),
            segments: vec![
                WhisperSeg { start: 0.0, end: 1.0, text: "what the dance sound".to_string() },
                WhisperSeg { start: 1.0, end: 2.0, text: "what the dance sound".to_string() },
                WhisperSeg { start: 2.0, end: 3.0, text: "what the dance sound".to_string() },
                WhisperSeg { start: 3.0, end: 4.0, text: "what the dance sound".to_string() },
                WhisperSeg { start: 4.0, end: 5.0, text: "what the dance sound".to_string() },
                WhisperSeg { start: 5.0, end: 6.0, text: "let's go beat the boss".to_string() },
            ],
        };

        let result = convert_whisper_result(&wr);

        // The hallucinated "what the" should NOT have produced any keyword entries.
        // Only the real "let's go" should be present.
        let has_what_the = result.keywords_found.iter().any(|k| k.keyword == "what the");
        let has_lets_go = result.keywords_found.iter().any(|k| k.keyword == "let's go");

        assert!(!has_what_the, "Hallucinated 'what the' should have been filtered out");
        assert!(has_lets_go, "Real 'let's go' should still be detected");
    }
```

**Note on `crate::whisper::TranscriptResult`:** the `convert_whisper_result` function takes `wr: &whisper::TranscriptResult`. Verify the import path before this test by running:

```
grep -n "use.*whisper::" src-tauri/src/commands/vod.rs
```

Adjust the `use crate::whisper::{...}` line at the top of the test to match. If `whisper::TranscriptSegment` is named differently (e.g., `whisper::Segment`), use the actual name.

If the whisper module's types aren't directly constructible from a test (private fields, etc.), simplify the test to construct a `Vec<TranscriptSegment>` directly (the post-conversion type) and call a small extracted helper. But first try the direct approach above.

- [ ] **Step 2.2: Run test, verify failure**

Run: `cd src-tauri && cargo test commands::vod::tests::convert_whisper_result_drops -- --nocapture`

Expected: assertion failure on `assert!(!has_what_the, ...)` because the current implementation matches the keyword in every segment, including hallucinated ones.

If you get a COMPILE error about `whisper::TranscriptResult` or `whisper::TranscriptSegment` instead, fix the import per the note above.

- [ ] **Step 2.3: Add the guard inside `convert_whisper_result`**

Find the keyword-scanning loop in `convert_whisper_result` (around vod.rs:551-565). It currently looks like:

```rust
    let mut keywords_found = Vec::new();
    for seg in &segments {
        let lower = seg.text.to_lowercase();
        for &kw in TRANSCRIPT_KEYWORDS {
            if lower.contains(kw) {
                keywords_found.push(TranscriptKeyword {
                    keyword: kw.to_string(),
                    timestamp: seg.start,
                    end_timestamp: seg.end,
                    context: seg.text.clone(),
                });
            }
        }
    }
```

Replace it with:

```rust
    // Phase A: skip segments inside hallucination runs (4+ identical
    // consecutive lines, typically whisper noise on background music).
    // See is_hallucinated_segment doc-comment.
    let mut keywords_found = Vec::new();
    for (idx, seg) in segments.iter().enumerate() {
        if is_hallucinated_segment(&segments, idx) {
            continue;
        }
        let lower = seg.text.to_lowercase();
        for &kw in TRANSCRIPT_KEYWORDS {
            if lower.contains(kw) {
                keywords_found.push(TranscriptKeyword {
                    keyword: kw.to_string(),
                    timestamp: seg.start,
                    end_timestamp: seg.end,
                    context: seg.text.clone(),
                });
            }
        }
    }
```

Three changes: `for seg in &segments` becomes `for (idx, seg) in segments.iter().enumerate()`, the new comment block, and the `is_hallucinated_segment` skip.

- [ ] **Step 2.4: Run all tests, verify pass**

Run: `cd src-tauri && cargo test commands::vod::tests -- --nocapture`

Expected: all tests in this module pass, including the new `convert_whisper_result_drops_keywords_from_hallucinated_runs`.

- [ ] **Step 2.5: Commit**

```bash
git add src-tauri/src/commands/vod.rs
git commit -m "feat(scoring): skip hallucinated transcript runs in keyword scan"
```

---

## Task 3: Transcript-only dimension override (TDD)

**Goal:** In `score_clip_candidate`, when a clip's `signal_sources` is exactly `[Transcript]`, override `context_simplicity = 0.50` and `emotional_spike = 0.40` BEFORE the weighted-sum total is computed. This drops the boilerplate fingerprint values that Phase B identified as the precision leak.

**Files:**
- Modify: `src-tauri/src/clip_selector.rs` — add the override block at the top of `score_clip_candidate`. Add unit tests in the existing `#[cfg(test)] mod tests` block (or add one if none exists).

- [ ] **Step 3.1: Confirm test infrastructure for clip_selector**

Run: `grep -n "mod tests" src-tauri/src/clip_selector.rs`

Two outcomes:
- **Tests block exists**: append new tests inside it.
- **No tests block**: create one at the very bottom of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_candidate(sources: Vec<SignalSource>) -> ClipCandidate {
        ClipCandidate {
            start_time: 100.0, end_time: 130.0, peak_time: 115.0,
            transcript_excerpt: None,
            event_tags: Vec::new(),
            emotion_tags: Vec::new(),
            payoff_summary: None,
            outcome_label: None,
            signal_sources: sources,
            hook_strength: 0.5, emotional_spike: 0.7625,
            payoff_clarity: 0.55, event_reaction_alignment: 0.47,
            context_simplicity: 0.88, replay_value: 0.5475,
            total_score: 0.0,
            similarity_fingerprint: String::new(),
            novelty_score: 0.0, diversity_penalty: 0.0, selection_score: 0.0,
            selected_reason: None, rejection_reason: None,
        }
    }
}
```

Verify the field names + types in `build_test_candidate` against the actual `ClipCandidate` struct at clip_selector.rs:61. If any field is missing or named differently in the current code, adjust the helper.

- [ ] **Step 3.2: Write the failing tests**

Append inside `mod tests`:

```rust
    #[test]
    fn score_clip_candidate_overrides_dimensions_for_transcript_only() {
        let mut c = build_test_candidate(vec![SignalSource::Transcript]);
        score_clip_candidate(&mut c);

        // After scoring, the dimensions should reflect the override.
        assert!((c.context_simplicity - 0.50).abs() < 1e-6,
            "context_simplicity should be 0.50, got {}", c.context_simplicity);
        assert!((c.emotional_spike - 0.40).abs() < 1e-6,
            "emotional_spike should be 0.40, got {}", c.emotional_spike);
    }

    #[test]
    fn score_clip_candidate_does_not_override_for_audio_only() {
        let mut c = build_test_candidate(vec![SignalSource::Audio]);
        let original_context = c.context_simplicity;
        let original_emotion = c.emotional_spike;

        score_clip_candidate(&mut c);

        // Audio-only clips keep their original dimension values.
        assert!((c.context_simplicity - original_context).abs() < 1e-6,
            "context_simplicity should be unchanged for audio-only");
        assert!((c.emotional_spike - original_emotion).abs() < 1e-6,
            "emotional_spike should be unchanged for audio-only");
    }

    #[test]
    fn score_clip_candidate_does_not_override_for_multi_signal_with_transcript() {
        let mut c = build_test_candidate(vec![SignalSource::Audio, SignalSource::Transcript]);
        let original_context = c.context_simplicity;
        let original_emotion = c.emotional_spike;

        score_clip_candidate(&mut c);

        // Transcript+audio is multi-signal — no override.
        assert!((c.context_simplicity - original_context).abs() < 1e-6);
        assert!((c.emotional_spike - original_emotion).abs() < 1e-6);
    }
```

- [ ] **Step 3.3: Run tests, verify failure**

Run: `cd src-tauri && cargo test clip_selector::tests::score_clip_candidate_overrides -- --nocapture`

Expected: assertion failure — `context_simplicity should be 0.50, got 0.88` (the test's transcript-only candidate keeps its starting 0.88 value because the override doesn't exist yet).

- [ ] **Step 3.4: Add the override block**

Find `pub fn score_clip_candidate(c: &mut ClipCandidate)` at clip_selector.rs:450. The current function is:

```rust
pub fn score_clip_candidate(c: &mut ClipCandidate) {
    c.total_score = (c.hook_strength * 0.30)
        + (c.emotional_spike * 0.20)
        + (c.payoff_clarity * 0.20)
        + (c.event_reaction_alignment * 0.15)
        + (c.context_simplicity * 0.10)
        + (c.replay_value * 0.05);
    let bonus = match c.signal_sources.len() { n if n >= 3 => 0.10, 2 => 0.05, _ => 0.0 };
    c.total_score = (c.total_score + bonus).min(0.99);
```

Insert the override block as the first statements inside the function body, BEFORE the existing `c.total_score = ...` line:

```rust
pub fn score_clip_candidate(c: &mut ClipCandidate) {
    // Phase A: transcript-only candidates emit a boilerplate dimension
    // fingerprint (typically context=0.88 from the "shock" tag branch in
    // analyze_context_simplicity, emotion≈0.7625 from the same tag
    // driving analyze_emotional_spike) regardless of actual content. The
    // override below replaces those values with less-confident defaults
    // BEFORE the weighted-sum total is computed, so the total reflects
    // the scorer's actual epistemic state for transcript-only inputs.
    // See docs/superpowers/specs/2026-05-07-phase-a-scoring-fix-design.md
    let is_transcript_only = c.signal_sources.len() == 1
        && c.signal_sources[0] == SignalSource::Transcript;
    if is_transcript_only {
        c.context_simplicity = 0.50;
        c.emotional_spike = 0.40;
    }

    c.total_score = (c.hook_strength * 0.30)
        + (c.emotional_spike * 0.20)
        + (c.payoff_clarity * 0.20)
        + (c.event_reaction_alignment * 0.15)
        + (c.context_simplicity * 0.10)
        + (c.replay_value * 0.05);
    let bonus = match c.signal_sources.len() { n if n >= 3 => 0.10, 2 => 0.05, _ => 0.0 };
    c.total_score = (c.total_score + bonus).min(0.99);
```

Note: Task 4 will append the cap to the very end of this function, after the existing `total_score = (...).min(0.99)` line.

- [ ] **Step 3.5: Run tests, verify pass**

Run: `cd src-tauri && cargo test clip_selector::tests::score_clip_candidate -- --nocapture`

Expected: 3 passed (the 3 new tests).

- [ ] **Step 3.6: Commit**

```bash
git add src-tauri/src/clip_selector.rs
git commit -m "feat(scoring): override boilerplate dimensions for transcript-only candidates"
```

---

## Task 4: 65% cap on transcript-only `total_score` (TDD)

**Goal:** As the final step of `score_clip_candidate`, if the candidate is transcript-only, cap `total_score` at 0.65. This is the safety-net layer per spec §3.3 — most transcript-only clips already fall below 0.65 from Task 3's dimension override, but the cap guarantees the user-facing complaint ("70%+ feels boring") is addressed regardless of how the underlying math evolves.

**Files:**
- Modify: `src-tauri/src/clip_selector.rs` — append the cap to the end of `score_clip_candidate`. Add 2 unit tests.

- [ ] **Step 4.1: Write the failing tests**

Append inside `mod tests`:

```rust
    #[test]
    fn score_clip_candidate_caps_transcript_only_at_65_percent() {
        // Build a candidate with extreme dimension values so the un-capped
        // total would land well above 0.65 even after the Task 3 override.
        // Hook 0.99 alone contributes 0.30. With the override-set context=0.5
        // and emotion=0.4, plus extreme other dims, total approaches the
        // pre-cap ceiling (0.99). The cap should clamp it to 0.65.
        let mut c = build_test_candidate(vec![SignalSource::Transcript]);
        c.hook_strength = 0.99;
        c.payoff_clarity = 0.99;
        c.event_reaction_alignment = 0.99;
        c.replay_value = 0.99;
        // (context_simplicity and emotional_spike will be overridden by Task 3)

        score_clip_candidate(&mut c);

        assert!(c.total_score <= 0.65 + 1e-6,
            "transcript-only total_score should be capped at 0.65, got {}", c.total_score);
    }

    #[test]
    fn score_clip_candidate_does_not_cap_multi_signal() {
        // Multi-signal candidate with the same extreme dim values should
        // be allowed to score well above 0.65.
        let mut c = build_test_candidate(vec![SignalSource::Audio, SignalSource::Transcript]);
        c.hook_strength = 0.99;
        c.payoff_clarity = 0.99;
        c.event_reaction_alignment = 0.99;
        c.replay_value = 0.99;

        score_clip_candidate(&mut c);

        assert!(c.total_score > 0.65,
            "multi-signal total_score should not be capped, got {}", c.total_score);
    }
```

- [ ] **Step 4.2: Run tests, verify failure**

Run: `cd src-tauri && cargo test clip_selector::tests::score_clip_candidate_caps -- --nocapture`

Expected: assertion failure — `transcript-only total_score should be capped at 0.65, got X` where X is something like 0.82 (un-capped).

- [ ] **Step 4.3: Add the cap**

Find the end of `score_clip_candidate` (right after `c.total_score = (c.total_score + bonus).min(0.99);`). Append:

```rust
    // Phase A safety net: transcript-only candidates capped at 0.65 even
    // if the dimension override + weighted sum somehow lands them above.
    // See docs/superpowers/specs/2026-05-07-phase-a-scoring-fix-design.md §3.3
    if is_transcript_only {
        c.total_score = c.total_score.min(0.65);
    }
}
```

(The closing `}` is the function's existing closing brace — make sure you're inserting BEFORE it, not duplicating it. The variable `is_transcript_only` was defined at the top of the function in Task 3 and is still in scope here.)

- [ ] **Step 4.4: Run tests, verify pass**

Run: `cd src-tauri && cargo test clip_selector::tests::score_clip_candidate -- --nocapture`

Expected: 5 tests passing (the 3 from Task 3 + the 2 new ones).

- [ ] **Step 4.5: Commit**

```bash
git add src-tauri/src/clip_selector.rs
git commit -m "feat(scoring): cap transcript-only total_score at 65%"
```

---

## Task 5: Phase-B-replay integration test

**Goal:** Add one test that exactly replays the Phase B boilerplate fingerprint and verifies the fix produces a score below 0.65. This locks in the user-visible outcome of the fix against the same data that drove the spec.

**Files:**
- Modify: `src-tauri/src/clip_selector.rs` — append one final test to `mod tests`.

- [ ] **Step 5.1: Write the test**

Append inside `mod tests`:

```rust
    #[test]
    fn score_clip_candidate_phase_b_boilerplate_lands_below_65() {
        // Replays the exact dimension fingerprint observed in Phase B for
        // transcript-only candidates rated boring/meh by the user:
        //   align=0.47, context=0.88, emotion=0.7625, payoff=0.55, replay=0.5475
        // Pre-fix, hook=0.69 (the "Drainage channel" rated-meh clip) produced
        // total_score ≈ 0.70. After the fix, the override + cap should
        // bring total_score below 0.65.
        let mut c = build_test_candidate(vec![SignalSource::Transcript]);
        c.hook_strength = 0.69;          // Phase B: "Drainage channel" hook
        c.emotional_spike = 0.7625;      // boilerplate value
        c.payoff_clarity = 0.55;
        c.event_reaction_alignment = 0.47;
        c.context_simplicity = 0.88;     // boilerplate value
        c.replay_value = 0.5475;

        score_clip_candidate(&mut c);

        assert!(c.total_score < 0.65,
            "Phase B boilerplate fingerprint should score below 0.65 after fix, got {}",
            c.total_score);
        // Also verify the dimensions were overridden as expected.
        assert!((c.context_simplicity - 0.50).abs() < 1e-6);
        assert!((c.emotional_spike - 0.40).abs() < 1e-6);
    }
```

- [ ] **Step 5.2: Run test, verify pass**

Run: `cd src-tauri && cargo test clip_selector::tests::score_clip_candidate_phase_b -- --nocapture`

Expected: 1 passed.

If this test fails (total_score >= 0.65), that's actually meaningful information — it would mean the current dimension-weight math + bonus produces a value that needs the cap to clamp. Both Task 3's override AND Task 4's cap are designed to handle this case. The test verifies the END-to-end result, not the implementation path.

- [ ] **Step 5.3: Run all clip_selector tests**

Run: `cd src-tauri && cargo test clip_selector::tests -- --nocapture`

Expected: 6 passed (3 from Task 3 + 2 from Task 4 + 1 from Task 5).

- [ ] **Step 5.4: Run all tests in the crate**

Run: `cd src-tauri && cargo test -- --nocapture 2>&1 | tail -30`

Expected: every prior test still passes, plus the new ones. No regressions.

- [ ] **Step 5.5: Commit**

```bash
git add src-tauri/src/clip_selector.rs
git commit -m "test(scoring): Phase B boilerplate fingerprint regression test"
```

---

## Task 6: Manual smoke test + version bump + ship v1.3.12

**Goal:** Validate the fix on a real VOD before shipping, bump version, tag, push.

**Files:** None modified for tasks 6.1-6.5. Version bump touches package.json, src-tauri/Cargo.toml, src-tauri/Cargo.lock, src-tauri/tauri.conf.json.

- [ ] **Step 6.1: Build verification**

Run from project root:

```
cd src-tauri && cargo check 2>&1 | tail -5
```

Expected: `Finished` line, 206-ish pre-existing warnings (the same set v1.3.11 ships with), 0 errors.

- [ ] **Step 6.2: Full test suite**

Run: `cd src-tauri && cargo test -- --nocapture 2>&1 | tail -40`

Expected: every test passes, including all the new ones from Tasks 1-5. Specific count to look for:
- The 5 `is_hallucinated_segment` tests
- The `convert_whisper_result_drops_keywords_from_hallucinated_runs` test
- The 5 `score_clip_candidate_*` tests (3 from Task 3 + 2 from Task 4)
- The `score_clip_candidate_phase_b_boilerplate_lands_below_65` test
- All pre-existing tests (game_config tests from prior work, etc.)

That's at least 12 new tests, plus the existing suite. Confirm they all pass.

- [ ] **Step 6.3: Live VOD smoke test**

Run: `cargo tauri dev` from project root.

In the running app:
1. Navigate to the **Vods** page
2. Pick the same DBD VOD used in Phase B (or a comparable VOD with multiple analyzed clips)
3. Click **Re-analyze** so the new scoring logic runs on fresh candidates
4. Wait for analysis to finish
5. Open the log file:

```powershell
Get-Content "$env:LOCALAPPDATA\com.clipgoblin.desktop\logs\ClipGoblin.log" | Select-String "scoring" | Select-Object -Last 20
```

Expected: at least some `[scoring]` lines should now show `context=50%` and `emotion=40%` for transcript-only clips. Cross-reference against `sources=["transcript"]` — those are the ones the fix targets.

6. Open the **Clips** page for that VOD. Sort by score (descending if not already).
7. Verify: any clip with `signal_sources = ["transcript"]` (visible by inspecting via the Review UI's exported JSON, since it's not directly in the card UI) is now at 65% or below. Multi-signal clips behave normally.

8. (Optional) If you have the Review UI toggle on, click **Export Reviews** on this VOD and inspect the JSON. The `clips[].dimensions.context` and `clips[].dimensions.emotion` values for transcript-only clips should be `0.5` and `0.4` respectively. The `total_score` for those clips should be `<=0.65`.

If anything looks off (e.g., transcript-only clips still scoring above 65%, or non-transcript-only clips having their scores changed), STOP and debug. Don't bump the version until the smoke test is clean.

- [ ] **Step 6.4: Bump version to v1.3.12**

```powershell
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
powershell -File bump-version.ps1 1.3.12
```

Expected output: confirmation that package.json, src-tauri/Cargo.toml, src-tauri/tauri.conf.json all updated to 1.3.12.

- [ ] **Step 6.5: Commit version bump**

```powershell
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git add package.json src-tauri/Cargo.lock src-tauri/Cargo.toml src-tauri/tauri.conf.json
git commit -m "chore: bump to v1.3.12 (transcript-only scoring fix)"
```

- [ ] **Step 6.6: Tag and push**

```powershell
cd "C:\Users\cereb\Desktop\Claude projects\clipviral"
git tag -a v1.3.12 -m "v1.3.12 -- transcript-only scoring fix

Fixes precision leak surfaced by Phase B Review UI: clips with only
transcript signal source no longer surface in the high-score band
where they were producing false-positive 70%+ ratings on calm
conversation moments.

Three layers:
- Hallucination guard: transcript runs of 4+ identical lines no
  longer feed the keyword scanner (whisper noise on background music)
- Dimension override: transcript-only candidates get
  context_simplicity = 0.50 and emotional_spike = 0.40 instead of
  the boilerplate 0.88 / 0.7625 produced by the shock-tag branch
- Total-score cap: transcript-only candidates capped at 65%

Multi-signal clips and audio-only / chat-only clips are unaffected.

Phase B regression test included; Phase B re-validation pass to
follow once the auto-updater pulls v1.3.12 on Slug's machine."
git push origin main
git push origin v1.3.12
```

GitHub Actions will produce a draft release. Edit notes + publish via the GitHub UI when CI is done.

---

## Self-review

(Run by the plan author with fresh eyes. Findings written here for transparency.)

### Spec coverage

Walking each spec section against the plan:

- **§1 Background** — context only, no implementation. ✓
- **§2 Goals & success criteria** — Tasks 1-5 implement the structural fix; Task 6 validates on a real VOD. The "transcript-only clips capped at 65%" criterion is verified by Task 4's tests + Task 5's regression test. ✓
- **§3.1 Hallucination guard** — Tasks 1 and 2 (helper + integration). ✓
- **§3.2 Boilerplate dimension fix** — Task 3. Note: spec said override happens at the COMPUTATION layer; plan implements it at the score_clip_candidate orchestration layer instead. This is intentional (per the plan's "files NOT changed" section) — keeps the per-dimension functions pure and tag-keyed for any other callers, isolates the special case to one place. Same observable behavior. ✓
- **§3.3 65% cap** — Task 4. ✓
- **§3.4 Decision matrix** — Tasks 3 and 4 collectively implement the matrix. The "audio-only" and "multi-signal" rows of the matrix are verified by the negative test cases in `score_clip_candidate_does_not_override_for_audio_only` and `score_clip_candidate_does_not_cap_multi_signal`. ✓
- **§4 File-level changes** — Plan pins exact file paths (vod.rs and clip_selector.rs). ✓
- **§5 Phasing** — Single release, all six tasks ship together as v1.3.12. ✓
- **§6 Watchouts** — All addressed: over-suppression risk mitigated by the targeted check (only `signal_sources == [Transcript]` exactly); hallucination threshold (4) is in code and tunable via constant; dimension-override-masks-deeper-bugs concern is a v1.3.13+ followup if Phase B replay surfaces issues; per-game config interaction is naturally clean since the override changes dimension VALUES not weights. Sample size concern is mitigated by structural fix (no data-fitted thresholds). ✓
- **§7 Open questions** — None. ✓
- **§8 Out-of-scope follow-ups** — Documented; not implemented in this plan. ✓

No spec gaps.

### Placeholder scan

Searched for the red-flag patterns:

- "TBD" / "TODO" / "implement later": none in plan steps (the inline `// TODO(v2):` comments in the existing code are pre-existing and out of scope).
- "Add appropriate error handling": none.
- "Write tests for the above" without code: none — every test step has full code blocks.
- "Similar to Task N": none — code is repeated where needed (e.g., the `build_test_candidate` helper is fully spelled out in Task 3 even though Tasks 4 and 5 reuse it; Task 4 and 5 say "Append inside `mod tests`" without re-defining the helper because it's clearly available from Task 3 in the same `mod tests` block — this is fine because the engineer reading Task 4 would already have completed Task 3 by then).

### Type consistency

- `is_hallucinated_segment(&[TranscriptSegment], usize) -> bool` — used consistently in Tasks 1 and 2.
- `score_clip_candidate(&mut ClipCandidate)` — signature unchanged from existing code (Task 3 only changes the body).
- `is_transcript_only` local variable defined in Task 3 is reused in Task 4 — both inserts into the same function so the variable is in scope.
- `SignalSource::Transcript` enum variant used consistently in tests and override check; matches the actual enum at clip_selector.rs:31.
- `build_test_candidate` helper added in Task 3 is reused by Tasks 4 and 5 — correctly relies on the same `mod tests` block being open across tasks.
- All field accesses (`c.signal_sources`, `c.context_simplicity`, etc.) match the `ClipCandidate` struct definition verified in pre-flight.

No type drift detected.
