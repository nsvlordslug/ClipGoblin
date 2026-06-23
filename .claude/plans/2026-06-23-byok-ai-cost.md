# BYOK AI Cost: visibility + cheap clip detection — Implementation Plan (2026-06-23)

**Goal:** Cut BYOK AI clip-detection token cost ~8–10× and make spend fully visible/predictable, *without* regressing clip quality.

**Build/verify reality:** Rust is NOT available in the Claude Code env — Slug runs `cd src-tauri && cargo check`, `cargo tauri dev`, and a real VOD analyze to verify each phase. Version-bump before any commit; never commit/push unless Slug asks (repo rules).

---

## What ALREADY exists (don't rebuild — verified via code map 2026-06-23)

- **Usage logging infra is solid.** `ai_usage::log_usage(&conn, UsageEntry)` (ai_usage.rs:88) → `ai_usage_log` table (db.rs:244). `compute_cost`/`token_cost_per_1k` (ai_usage.rs:40–82) already price Haiku (`in 0.0008 / out 0.004` per 1k) and Sonnet (`0.003 / 0.015`); unknown→Sonnet.
- **The clip judge ALREADY logs** — `vod.rs:1811` writes a `"clip_judge"` row with tokens_in/out after `run_ai_judge`.
- **Caption/title commands log** — `commands/captions.rs` logs at 5 sites (title_save, money_quote_title/caption, caption_regen, title_regen).
- **In-app spend display exists** — `Settings.tsx:544–569` shows avg-per-analyze + 30-day total via `get_ai_cost_summary` (settings.rs) → `estimate_cost()` (ai_usage.rs:133, returns `CostSummary{avg_per_analyze_usd, total_30d_usd, vod_count}`).
- **Candidate-windowing exists** — `clip_selector::select_candidate_windows(...) -> Vec<(f64,f64)>` (clip_selector.rs ~889) → `run_windowed_transcription_native(..., &candidate_windows, ...)` (vod.rs ~1937). Judge gets the resulting (possibly windowed) `TranscriptResult`.
- **Judge model resolution** — `ai_provider::resolve(&conn, Scope::ClipJudge)` (ai_provider.rs:170) → default `claude-sonnet-4-6` (`default_claude_model()` line 135).
- **Candidate structs** — `RawSignal`/`FusedMoment`/`ClipCandidate` (clip_selector.rs 19–91). `extract_transcript_for_range(transcript, start, end)` (vod.rs:1184) already pulls text for a time range.

**Net:** the scaffolding is here. The cost is high because (a) candidate windows merge to ≈full-VOD so the judge reads everything, and (b) the judge runs on Sonnet. Visibility is ~80% there; gaps are a pre-run estimate + prominence + any un-logged path.

---

## Phase 1 — Visibility (small, low-risk, additive). Do first = instrumentation for proving Phase 2.

### Task 1.1 — Confirm + close logging gaps
- **Verify** whether analysis-time titles/captions via `post_captions.rs` (`generate_llm` / `generate_llm_title`, called from the Wave-3 upgrade path) write a `log_usage` row. The map flagged these as caller-decides; the dev-run "Save-path Wave 3" went through `commands/captions.rs` (which logs) — confirm there's no path that calls Claude without logging.
- **Verify** `vision_signal.rs` (any live Claude call) logs; add `log_usage` if it bills and doesn't.
- Pattern to replicate is the existing `commands/captions.rs` post-call `log_usage(&conn, UsageEntry{ feature, provider, model, tokens_in, tokens_out, vod_id, clip_id, context })`.
- **Verify (Slug):** run an analyze, then check `ai_usage_log` has rows for `clip_judge` + the analysis-time titles for that `vod_id`.

### Task 1.2 — Pre-run cost estimate (the genuinely missing piece)
- Before Analyze, show "Est. ~$X this analysis" so cost is never a surprise.
- Simplest accurate source: reuse `estimate_cost()`'s `avg_per_analyze_usd` (avg of last N analyses) → show that as the estimate when history exists; fall back to a duration-based projection for the first run (≈ judge tokens scale with VOD length; titles/captions ≈ per-clip × expected clips).
- New tiny command `estimate_analyze_cost(vod_duration_secs, model) -> f64` in settings.rs, or extend `get_ai_cost_summary`. Render on the VOD card / Analyze button area (`Vods.tsx` / wherever Analyze is triggered).

### Task 1.3 — Surface per-analysis cost after a run
- After analyze completes, show "This analyze cost ~$Y" in the completion toast/notification (sum `ai_usage_log.cost_usd WHERE vod_id = ?`). Keep the Settings 30-day display too.

---

## Phase 2 — Cheap clip detection (the real ~8–10× win). Touches the detection core → Slug reviews this section before I edit.

### Task 2.1 — Default the JUDGE to Haiku (titles/captions unchanged)
- Add a judge-specific model default: in `ai_provider::resolve()` branch on `Scope::ClipJudge` → default `claude-haiku-4-5-20251001` instead of `default_claude_model()`.
- Add an optional `claudeJudgeModel` to `AiSettings` (aiStore.ts) + a dropdown (default Haiku) so power users can force Sonnet. Titles/captions keep following `claudeModel`.
- Cost: ~3× off the judge immediately, independent of 2.2.

### Task 2.2 — Cap the judge's input to top-N candidate snippets (the big cut)
- Today the judge prompt is built from the whole (merged-window) transcript via `build_transcript_text(transcript)` (clip_judge.rs:252). Instead, feed the judge the **top-N highest-signal candidate windows** (N≈20–30), each as a compact snippet: `[mm:ss] <±20–30s text via extract_transcript_for_range> (signals: tags)`.
- Source candidates from the fused signal set (`FusedMoment`/`ClipCandidate`, ranked by intensity/score) — pass them into `judge()` (new arg) and build the prompt from snippets, not the full transcript. Judge returns scores keyed to candidate timestamps (re-rank/validate), which then feed the existing fusion (`select_clips`, vod.rs:2001) via `ClipCandidate.ai_score`.
- Keep the net **generous** (N high) to protect recall; this is far fewer tokens than the full transcript even at N=30 because we drop dead air + non-candidate spans.
- **Confirm** `select_candidate_windows` merge behavior; the judge cap should hold regardless of how transcription windowed.

### Task 2.3 — Sonnet final-pass on the top survivors (taste, cheap)
- After Haiku ranks the N candidates, take the top ~8–10 and run **one Sonnet call** that picks/orders the final clips (reads ~10 short snippets, not the VOD). Toggle `useSonnetFinalPass` (default on). Net judge cost ≈ Haiku-bulk + small-Sonnet ≈ still ~4–5× cheaper than today, near-Sonnet taste.

### Task 2.4 — Transcript cleanup before judging
- Pre-clean transcript segments before building snippets: collapse whisper repetition-loops (e.g. "wait, wait, wait…" ×N, "I'm not sure." ×N — these appeared verbatim in the 2026-06-22 run), drop `[_TT_]`-style/empty artifacts, dedupe adjacent identical segments, trim dead air. Cuts tokens ~20–40% AND improves judgment. Implement as a `clean_segments(&[TranscriptSegment]) -> Vec<TranscriptSegment>` used by the snippet builder (and optionally SRT).

### Task 2.5 — (Minor) tighten judge output
- Prompt already returns compact JSON (`{start,end,category,score,reason}`). Optionally shorten/omit `reason` to cut output tokens (5× input cost). Low priority.

---

## Validation (the proof)
With Phase 1 logging live, Slug analyzes the SAME VOD before and after Phase 2 and compares `SUM(cost_usd) WHERE vod_id=? AND feature='clip_judge'`. Target: ~7¢ → ~1–2¢, with clip set quality equal or better (eyeball the clips).

## Quality guardrails (from the design Q&A)
- Recall now routes through the candidate net → keep N generous; a cheap text-interestingness signal can be added later to catch zero-signal text gems.
- Haiku taste gap is closed by the Sonnet final-pass (2.3).
- Detection quality of Phase 2 ≈ today's, at ~1/8 the cost; the architecture is also the platform for the planned per-creator + edit-feedback learning.
