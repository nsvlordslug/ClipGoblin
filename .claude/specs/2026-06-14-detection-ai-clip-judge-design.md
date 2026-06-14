# Detection Quality: Corroboration Gate + AI Clip-Worthiness Judge — Design

**Goal:** Stop the detector from equating *loudness* with *quality* — it currently over-rates loud-but-empty moments (laughing while explaining OBS → 91%) and misses quiet content (deadpan banter/roasts). Replace "loud = good" with "is this one of the things the creator actually clips."

**Architecture:** Two coordinated changes. (1) A free, signal-level **corroboration gate** on the audio loudness boost so a loud spike only counts when an independent signal backs it. (2) An opt-in, BYOK **AI clip-worthiness pass** that reads the timestamped transcript, *discovers* clip-worthy moments the signals missed, and *judges* the ones they found — fused as the primary ranker. Off by default; cost shown in Settings before it's enabled.

**Tech stack:** Rust (`clip_selector.rs`, `commands/vod.rs`, new `clip_judge.rs`), existing BYOK plumbing (`ai_provider.rs` resolve, `ai_usage.rs` cost), React Settings UI.

**Status:** DESIGN — discovered during v1.5.0 calibration validation. Follows `.claude/specs/2026-06-14-detection-calibration-design.md`. Branch `feat/detection-calibration-v1.5.0`.

---

## 1. Problem (root-caused 2026-06-14)

Real over-rated clip, VOD `is that chucky? no its Bynter!` @ 2224s, scored **91.1%**:
> *"(laughing) So it was okay, when you do a shared stream, you get a separate chat window that shares the chat between all three of you..."*

Score anatomy from the persisted `scoring_dimensions`:
- 6 content dimensions sum to **0.57** (hook 0.60, emotion 0.73; but payoff 0.47, replay 0.40).
- The **z-envelope loudness boost adds +0.34** → 0.91.

Three independent loudness pathways triple-count one laugh: `analyze_hook_strength` (audio ratio), `analyze_emotional_spike` (`best_intensity*0.45` + "got loud" bonus), and the z-boost in `select_clips`. The two dimensions that mean "something happened" — payoff, replay — are only 25% of the weight and cannot veto.

**Unifying root cause: loudness ≈ score.** This single cause produces BOTH user complaints:
- Loud laughter over logistics → **over**-scored (91%).
- Quiet deadpan banter/roasts → **under**-scored, never even generates a candidate.

Signals alone cannot separate "laughing because a joke landed" from "laughing while explaining settings" — that distinction is **semantic**.

## 2. Clip-worthiness criteria (creator-confirmed)

The AI judge's instruction set (all four are in-scope for this creator's content):
1. **Funny banter & roasts** — friends ribbing each other, jokes landing, savage one-liners, *including deadpan/quiet ones with no audio spike*.
2. **Big plays & clutches** — skillful gameplay, clutch escapes/saves, wins, outplays.
3. **Scares & big reactions** — jumpscares, genuine shock, panic, rage.
4. **Hype & group energy** — collective "OH MY GOD", celebrations, squad chaos.

**Anti-criteria (explicitly NOT clips):** explaining settings/OBS/logistics, dead air, mic checks, "what video are you watching" housekeeping — regardless of volume or laughter.

## 3. Piece 1 — Corroboration gate (signal-level, free, always on) ✅ IMPLEMENTED (1131a5b)

> **Validated change vs. the draft below.** Uncorroborated boost is CAPPED at `0.12`, NOT scaled — a scale (×0.35) penalizes every single-signal clip equally and re-starved "You sound big" 6→3; a cap only trims BIG bare spikes (a loud laugh), sparing modest single-signal boosts. Corroboration = `≥2 sources OR Community` only — keyword tags are excluded because `shock` is over-applied (lands on mundane OBS chatter). The **emotion-dimension gate was DROPPED** (same re-starve risk, unvalidatable from stored data; the cap already demotes ambient laughter ~90→70 for free users). Validated result: laugh clip 90→70 and no longer top, both VODs hold their clip counts.

The z-envelope boost in `select_clips` currently applies to every candidate by its peak audio z. Change it to scale by **corroboration**: a loud spike earns the full boost only when an *independent* signal backs it; a bare loud laugh earns a fraction.

- **Corroborated** when ANY of: `signal_sources.len() >= 2` (genuine multi-signal) OR the moment carries a hard-event tag (`kill`/`clutch`/`win`/`jumpscare`/`scream`/`death` — NOT soft `hype`/`reaction`/`laughter`).
- **Boost = `base_boost * factor`**, `factor = 1.0` corroborated, `~0.35` uncorroborated. (Exact factor tuned in validation.)
- **Effect:** the 91% laughter clip → ~0.62 (its honest content score, no longer top). Loud-stream fix preserved: real moments are corroborated and keep the full boost ("You sound big" must not re-starve).

This is necessary but not sufficient — it de-fangs raw loudness but cannot tell mundane laughter from a landed joke. Piece 2 does that.

## 4. Piece 2 — AI clip-worthiness pass (semantic, BYOK, opt-in)

### 4.1 Trigger & gating
- New setting **`ai_clip_detection_enabled`** (bool, **default false**).
- Disabled/greyed when no BYOK provider is configured (`Provider::Free`) — falls back to signal-only.
- When enabled: runs once per `analyze_vod`, after signal candidates exist, before final selection.

### 4.2 The call
- Build a **timestamped transcript** (`[mm:ss] text` per segment) from `TranscriptResult.segments`.
- One consolidated call via the resolved BYOK provider (`ai_provider::resolve`). Prompt = §2 criteria + anti-criteria + "only reference timestamps present below."
- **Long-VOD guard:** if the transcript exceeds a token budget (~100k), split into ≤N sequential windowed calls and merge. Typical 99-min VOD = one call.
- **Output (structured JSON):** `[{ start_sec, end_sec, category, score (0-100), reason }]`. Validate every timestamp against real segment bounds; clamp/drop hallucinated ranges.
- **Log usage** via `ai_usage::log_usage` (feature `"clip_judge"`, the VOD id) so it appears in the existing cost reports.

### 4.3 Fusion (AI as primary ranker)
- Each candidate carries `signal_score` (existing 0–1) and, when AI ran, `ai_score` (0–1 from the judge).
- **AI-discovered** moments with no signal candidate become new candidates (signal source `Semantic`), run through boundary optimization + gates like any other.
- **Blend when AI ran:** `final = 0.65*ai_score + 0.35*signal_score` (starting point, tuned in validation). This lets the AI **rescue** (quiet banter: high ai, low signal) and **veto** (mundane laughter: low ai, mid signal → demoted out).
- **AI off / unavailable / failed:** `final = signal_score` (Piece 1 still applies). Graceful, logged, never blocks analysis.

### 4.4 Cost in Settings (the creator's ask)
- Next to the toggle, show an **estimated cost per analysis** computed from `ai_usage::compute_cost(resolved.provider, resolved.model, ~18_000, ~1_500)` — i.e. the same pricing table the rest of the app uses — refined to the rolling real average (`estimate_cost`) once analyses exist.
- Reuse the **existing pre-analyze confirmation modal** to surface the AI cost the first time a run will incur it.
- Example line: *"AI clip detection — adds ≈ $0.02 per VOD (Claude Haiku). Uses your configured AI provider."*

## 5. Data flow

```
transcript + audio + chat
        │
        ├─ signal candidate generation ──────────────┐
        │                                            │
   [if ai_clip_detection_enabled]                    │
        │                                            ▼
   clip_judge::judge(transcript) ──► AI moments ► fuse into candidate pool
        │                                            │
        └─ ai_score attached to overlapping cands    │
                                                     ▼
        Piece 1 corroboration-gated boost ► score_clip_candidate ► fusion blend
                                                     ▼
                  apply_two_gate_selection (scene-card guard + floor + top-K)
                                                     ▼
                                              highlights
```

## 6. Code units
- `clip_selector.rs` — corroboration gate in the z-boost site; `fn is_corroborated(c)`; fusion blend when `ai_score` present; `ClipCandidate.ai_score: Option<f64>`, `SignalSource::Semantic`.
- `clip_judge.rs` (NEW) — `judge(transcript, provider) -> Vec<JudgedMoment>`; prompt builder; JSON parse + timestamp validation; long-VOD windowing.
- `commands/vod.rs` — read the setting; call the judge; fuse moments → candidates; pass `ai_score` through; cost logging.
- `commands/settings.rs` + React Settings — the toggle, the BYOK gate, the estimated-cost line; pre-analyze modal wiring.
- `ai_usage.rs` / `ai_provider.rs` — reused as-is (no change expected beyond a feature label).

## 7. Acceptance criteria (validate before the creator re-analyzes)
- **A. Over-rate fixed:** `is that chucky?` @2224s ("shared stream" laughter) drops well below the gameplay clips (target < 0.55 with AI on; not top-ranked with AI off).
- **B. Banter surfaced:** with AI on, at least the creator-named deadpan/banter moments become candidates and rank.
- **C. No regression:** `You sound big` (the loud VOD) still yields its healthy gameplay set (≥4, no re-starve); scene cards still excluded.
- **D. Cost honest:** Settings shows a per-VOD estimate matching `compute_cost` for the configured model; free-tier shows the toggle disabled.
- **E. Safe fallback:** AI off / no key / API error → signal-only path, analysis still completes.

## 8. Build sequence
1. **Piece 1** corroboration gate (TDD) → unit tests → confirm 91% clip math drops → commit.
2. **Piece 2a** `clip_judge.rs` (prompt, call, parse, validate) behind the setting (default off) → tests with a stubbed provider → commit.
3. **Piece 2b** fusion + candidate injection + cost logging → tests → commit.
4. **Settings UI** toggle + cost line + BYOK gate → typecheck → commit.
5. **Validation pass:** re-run both VODs (acceptance A–E), tune the gate factor + fusion weights, commit tuning.

## 9. Out of scope / risks
- **Out:** per-creator AI memory, edit-feedback learning (later layers); visual/facecam emotion; game-state CV.
- **Risk — AI cost surprise:** mitigated by default-off + Settings estimate + pre-analyze modal.
- **Risk — AI hallucinated timestamps:** mitigated by validating every returned range against transcript segments.
- **Risk — fusion over-trusts AI** (misses audio-only moments with no speech, e.g. a wordless scream): the 0.35 signal weight + signal-discovered candidates keep audio-only moments alive; tuned in validation.
- **Risk — long VODs blow the context window:** windowed multi-call fallback above a token budget.
