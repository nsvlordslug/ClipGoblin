# Phase A: Transcript-Only Scoring Fix — Design

**Status:** Draft for review
**Date:** 2026-05-07
**Target release:** v1.3.12 (user-facing scoring improvement)

---

## 1. Background

After v1.3.11 shipped (per-game detection configs), user testing surfaced a precision problem: clips scoring 70%+ don't reliably correspond to entertaining moments. Phase B (the dev-only Review UI shipped under the `showReviewTools` Settings toggle) gathered ground-truth ratings on two VODs of different content types and confirmed a structural cause:

> **Transcript-only candidates emit a boilerplate dimension stack (`context=0.88, emotion=0.7625, align=0.47, payoff=0.55, replay=0.5475`) regardless of what was actually said.** With `hook_strength` as the only meaningful variable, total scores cluster in the 65–70% band. Result: every transcript-only clip — whether actual banter or calm chatter — looks identically "70%-ish" in the ranking.

Evidence summary from Phase B:
- DBD VOD: 8 of 8 transcript-only clips rated meh or boring; 0 rated good
- Minute (cozy) VOD: 2 of 2 transcript-only clips had identical dimensions despite one being rated good and one rated meh
- Across both VODs, every transcript-only clip had the same five-dimension fingerprint above
- The transcript-only clips also shared a tag (`shock`) that fired on whisper-hallucinated music output (e.g. `(little bit of a "dance" sound)` repeated 16 times being tagged as a shock event)

Multi-signal candidates (any combination not equal to `[transcript]`) had varied dimensions and could be either good or boring depending on content — they're not the precision leak.

## 2. Goals & success criteria

### Goal
Fix the precision leak so users no longer see clips at 70%+ that are obviously not clip-worthy. Specifically: transcript-only candidates should not surface in the high-score band where they currently dominate the false-positive set.

### Success criteria
- Re-running Phase B's review pass on the same DBD VOD produces fewer 70%+ rated boring/meh
- Transcript-only clips' total scores cap at 65%
- Transcript-only clips' dimension breakdowns reflect the scorer's actual epistemic state (i.e. not the boilerplate `context=0.88` values that imply confidence the scorer doesn't have)
- Whisper-hallucinated transcript output (4+ identical consecutive lines) no longer registers as transcript signal at all
- No regression on multi-signal good clips — clips like the "Halle Tree Pomodgranate" (95% multi-signal good) keep their high scores
- The fix applies regardless of per-game config (since both Phase B VODs ran on default config, the structural fix has to work without per-game tuning)

### Out of scope for this spec
- Broader whisper post-processing cleanup (fixing `(little bit of a "dance" sound)` from being produced in the first place — that's a transcription-pipeline issue, not a scoring issue)
- Multi-voice / laughter audio classifier (long-term improvement; not needed for Phase A)
- Tag system overhaul (the `shock` tag firing on noise is a downstream symptom of bad transcript input — fixed at the input layer here, not at the tag layer)
- Per-game scoring weights (Phase A's structural fix works on default config; per-game refinement is orthogonal and lives in v1.3.13+)

## 3. Design

Three layers, applied in order during scoring:

### 3.1 Hallucination guard (input layer)

**Problem:** Whisper sometimes produces repetitive nonsense for VOD audio that's actually background music or low-information ambient sound. Phase B exhibit: `(little bit of a "dance" sound)` 16× consecutive in two clips. This nonsense passes through the transcript signal pipeline and registers as transcript signal, which in turn lets clips qualify as "multi-signal" when paired with audio (defeating any rule that checks `signal_sources`).

**Fix:** When finalizing transcript signal segments, count consecutive identical lines within a candidate window. If any window has 4 or more identical consecutive lines, treat that window as having no usable transcript signal — i.e. the transcript signal contribution to that candidate is dropped.

**Effect on `signal_sources`:** A clip whose transcript was 4+ repeated lines now has `signal_sources` that does NOT include `Transcript`. So a clip that previously had `[audio, transcript]` (with hallucinated transcript) becomes `[audio]` only — which is fine, it's still real audio signal.

**Threshold (4 consecutive identical lines):** chosen to not false-positive on legitimate repetition (e.g. a streamer chanting "go go go go"). 4+ is a strong signal of nonsense. Tunable later if needed.

### 3.2 Boilerplate dimension fix (computation layer)

**Problem:** When a `ClipCandidate` has `signal_sources == [Transcript]`, the existing scoring logic produces the same five-dimension stack regardless of transcript content: `context=0.88, emotion=0.7625, align=0.47, payoff=0.55, replay=0.5475`. Only `hook_strength` varies (slightly) by transcript content. The boilerplate values reflect "we don't have a real way to compute these for transcript-only candidates so we hardcoded confident-looking defaults."

**Fix:** When a `ClipCandidate` has `signal_sources` equal to exactly `[Transcript]` (after the hallucination guard runs), override `context_simplicity` and `emotional_spike` to lower, less-confident values:
- `context_simplicity`: 0.50 (was 0.88) — "we don't know if this transcript is contextually clear"
- `emotional_spike`: 0.40 (was 0.7625) — "we don't have audio or chat to confirm emotional intensity"

Other dimensions (`hook_strength`, `payoff_clarity`, `event_reaction_alignment`, `replay_value`) keep their existing computation. They're already varied by content (`hook_strength` more than the others), and their boilerplate values aren't as clearly inflated.

**Why these two specifically:** Both `context_simplicity` and `emotional_spike` register as `0.88` and `0.7625` for every transcript-only candidate in Phase B's data. They're the only two that demonstrably ignore content. The other dimensions either already vary or are at lower values that don't drive the false-positive precision leak.

**Effect on total score:** With these reduced dimension values feeding into the existing weighted-sum total computation, the typical transcript-only total drops from ~0.65 to ~0.45-0.50 organically. Most transcript-only clips fall below the 70% threshold without needing an explicit cap.

### 3.3 65% cap for transcript-only (output layer / safety net)

**Problem:** Even after 3.2's dimension fix, edge cases exist where some other dimension (e.g. an unusually high `hook_strength` from interesting transcript content, or a future scoring change that adds dimension weight) could push a transcript-only total above 70%. The user complaint was specifically "70%+ feels boring" — capping at 65% guarantees the user-facing complaint is addressed regardless of internal scoring drift.

**Fix:** As the final step of total-score computation, if `signal_sources == [Transcript]`, apply `total_score = total_score.min(0.65)`.

**Why a hard cap:** Cheaper than scaled multipliers (no magic number to tune), easier to reason about ("transcript-only ≤ 65% always"), and acts as a guarantee independent of how the dimension math evolves. Most of the time it's a no-op because 3.2 already pulled the total below 0.65.

**Where in the pipeline:** Applied UPSTREAM of clip selection, so it affects which clips get picked from the candidate pool — not just the displayed score. Per Slug's "keep ranking honest" call: capping only the displayed score would mean the same boring clips still get picked, just shown with lower numbers. We want them to actually rank lower in the candidate ordering.

### 3.4 Decision matrix

| `signal_sources` value | Hallucination guard active? | Dimension fix active? | Cap active? |
|---|---|---|---|
| `[Transcript]` (real) | filters at 4+ repeats | yes (context→0.50, emotion→0.40) | yes (≤0.65) |
| `[Transcript]` (hallucinated) | becomes `[]` (no signal) | n/a (clip filtered out earlier) | n/a |
| `[Audio]` only | n/a | no | no |
| `[Chat]` only | n/a | no | no |
| `[Audio, Transcript]` | filters at 4+ repeats; if hallucinated, becomes `[Audio]` | no | no |
| Any other multi-signal | n/a | no | no |

The fix is targeted: only the `[Transcript]`-only path is touched. All other configurations behave identically to today.

## 4. File-level changes

Concrete file mapping deferred to the implementation plan, but the logical layers map roughly to these areas of the codebase:

- **Layer 3.1 (hallucination guard)** — likely in `src-tauri/src/transcript_signal.rs` or wherever the transcript signal pipeline ingests whisper output and produces signal segments
- **Layer 3.2 (dimension fix)** — likely in `src-tauri/src/clip_selector.rs` or wherever `ClipCandidate.context_simplicity` / `.emotional_spike` are computed
- **Layer 3.3 (65% cap)** — at the same location as 3.2, applied after `total_score` is computed but before the clip is added to the candidate pool

The plan will pin exact line numbers and TDD test structure.

## 5. Phasing

Single release, single user-facing v1.3.12. No sub-phasing — the three layers are tightly coupled (hallucination guard protects the dimension fix; dimension fix sets up natural ranking; cap is the safety net). All ship together.

After v1.3.12 ships:
- Slug repeats the Phase B review pass on the same DBD VOD (or comparable content)
- Compare: are clips at 70%+ now consistently rated good/meh, with boring clips landing below 70%?
- If yes: ship as v1.3.12, declare investigation closed
- If precision is still leaky: open Phase C investigation with the new data

## 6. Sparring-partner watchouts

- **Risk: 3.2 over-suppresses good transcript-only clips.** Audio-only good clips exist (Florida swamp at 72% in Phase B data was audio-only and rated good). Are there transcript-only good clips? Phase B had zero in 8 ratings, so the empirical answer is "no" — but the sample is small. If a future user has a Just Chatting VOD where every moment is transcript-only and one is genuinely funny, the cap suppresses it. Acceptable trade-off given (a) the user complaint is explicit about transcript-only clips being false positives, and (b) the cap doesn't ELIMINATE them, just keeps them below 65%, so they still surface in the candidate pool, just lower-ranked.

- **Risk: hallucination threshold (4+ repeats) is too aggressive or too lax.** Too aggressive (e.g., 2+) would flag legitimate repetition (chanting, songs, repeated callouts in tense gameplay). Too lax (e.g., 8+) would miss shorter hallucination bursts. 4 is a reasonable middle. If Phase B+ data surfaces false positives or false negatives, easy to tune.

- **Risk: the dimension fix masks a deeper problem.** If `context=0.88` was really computed by some logic (not just hardcoded), our override may break something downstream that depends on that computation. The plan needs to verify whether the boilerplate value comes from a hardcoded constant or a computation that returned 0.88 for these specific inputs. If computed, we may need to dig deeper.

- **Risk: per-game configs interact with the fix unexpectedly.** Per-game `transcript_weight` (0.7 for horror, 1.3 for rpg, etc.) currently re-weights how transcript signal contributes to total. Our fix changes the dimension VALUES, not the weights, so the interaction should be clean (lower dimension × any weight = still lower). But if `transcript_weight=1.3` (rpg) somehow inflates a fixed dimension above the cap, the cap catches it. No conflict expected.

- **Risk: Phase B sample size (2 VODs, ~18 clips rated total) is thin.** Pattern matching on small data risks tuning to specific examples. Mitigation: the fix is structural (hallucination guard + dimension override + cap) rather than data-fitted (no specific score thresholds tuned to Phase B clips). The fix targets a STRUCTURAL behavior (boilerplate dimensions for transcript-only) that's evident even from the small sample. Re-test post-v1.3.12 will validate.

## 7. Open questions

None at the time of writing. All design decisions resolved during the brainstorming Q&A:
- Q1 (where to apply): upstream, affects selection
- Q2 (cap value): 65%
- Q3 (what triggers the rule): exactly `signal_sources == [Transcript]`
- Q4 (dimensions vs cap): both
- Hallucination handling: include narrow guard (4+ identical repeats) in Phase A scope

## 7a. Post-smoke-test scope expansion (added 2026-05-08)

After Tasks 1-5 landed and the implementer ran a smoke-test re-analysis on the same DBD VOD that drove the original spec, two additional precision leaks surfaced in the data:

- **Audio-only clips with shock-family tags emit the same boilerplate `context=0.88` as transcript-only clips.** The original `analyze_context_simplicity` returns 0.88 for ANY clip with `shock`/`jumpscare`/`scream`/`surprise` tags, regardless of signal source. Phase B happened to surface this issue mostly via transcript-only clips because of how the keyword detector tagged them, but the underlying issue is broader: `generate_audio_candidates` also adds those tags for certain audio-peak patterns. Result: boring audio-only "Chat about games" (74%) and "Soda flavor" (70%) clips persist above the 65% threshold even after the original fix.
- **`keyword_boost_for_range` adds up to +0.20 based on TRANSCRIPT_KEYWORDS matches.** The current keyword list includes conversational words (`let's go`, `what the`, `run`, `help`, `behind`, `dead`, `done`, `yes`, `dude`, `bro`) that appear constantly in calm gaming chatter. Result: "Build talk" (78%, audio+transcript) gets a kw_boost from "let's go" appearing in a calm strategy discussion — pushing a multi-signal-but-uninteresting clip into the high-score band.

### 7a.1 Layer 4: generalize the override + cap predicate

The existing `is_transcript_only = signal_sources == [Transcript]` predicate becomes:

```rust
let has_shock_family_tag = |c: &ClipCandidate| -> bool {
    let tag_check = |t: &str| matches!(t, "shock" | "jumpscare" | "scream" | "surprise");
    c.event_tags.iter().any(|t| tag_check(t.as_str()))
        || c.emotion_tags.iter().any(|t| tag_check(t.as_str()))
};
let is_unreliable_single_signal = c.signal_sources.len() == 1
    && has_shock_family_tag(c);
```

This catches:
- Transcript-only with shock tag (the original Phase A target)
- Audio-only with shock/jumpscare tag (the new audio-only false positives)
- Chat-only or community-clip-only with these tags (hypothetical, but covered for completeness)

The override (`context = 0.50, emotion = 0.40`) and the 65% cap apply to the broader set. The same persist-time cap at `vod.rs:1902` uses the same predicate.

**Trade-off:** borderline-good audio-only clips with shock-family tags get suppressed too. Specific Phase B example: the "Florida swamp" clip (audio-only, `ambush,jumpscare,shock` tags, rated 'good' but borderline — a 23-second mild map-name joke) drops from 72% to ~61%. Decision (per Slug 2026-05-08): generalize anyway. The clip is still in the candidate pool, just ranked below multi-signal good clips. The user-facing UX gain (no boring clips at 70%+) outweighs the loss on borderline single-signal clips.

### 7a.2 Layer 5: tighten TRANSCRIPT_KEYWORDS

Drop conversational words from the list at `commands/vod.rs:533`. Keep only words that genuinely indicate strong reactions:

**Keep** (8 words): `"no way", "oh my god", "holy", "clutch", "rage", "noooo", "nooo", "oh no"`

**Drop** (10 words): `"what the", "let's go", "lets go", "run", "help", "behind", "dead", "done", "yes", "dude", "bro"`

These conversational words appear constantly in calm gaming speech and produce false-positive kw_boost. The 8 kept words are exclamation-shaped and rarely appear in calm planning.

**Rationale for not relying on keyword absence alone:** real "let's gooo!" reactions fire audio peaks AND chat reactions, so they'll still score high through the dimensions + multi-signal bonus + audio peak signal. The kw_boost was the wrong tool to detect them — audio analysis is.

**Risk:** false negatives — a real strong reaction using only the dropped words and no audio peak might miss the boost. Mitigation: rare in practice; user can rate it good in the Review UI for future tuning iterations.

### 7a.3 Decision matrix (updated)

| Configuration | Original Phase A behavior | Post-amendment behavior |
|---|---|---|
| `[Transcript]` + shock tag | Override + cap fires | Same (covered by new predicate) |
| `[Transcript]` + only "speech" tag | Override + cap fires | Override + cap does NOT fire (no shock-family tag) — but those clips don't have the boilerplate fingerprint anyway |
| `[Audio]` + shock/jumpscare tag | NOT covered | **Override + cap fires** (NEW) |
| `[Audio]` + chase/encounter/hype tag | NOT covered | NOT covered (different context_simplicity branch, not the boilerplate) |
| `[Audio, Transcript]` + any tag | NOT covered | NOT covered (multi-signal preserves) |
| Any multi-signal | NOT covered | NOT covered |

### 7a.4 What ships in v1.3.12

The amendment scope ships in the SAME v1.3.12 release as the original Phase A fix (Tasks 1-5). Not a separate release. After all three layers (hallucination guard + dimension override/cap + keyword tightening) land, Slug runs the smoke test with all of them active, then version-bumps + tags + ships.

## 8. Out-of-scope follow-ups for v1.3.13+

- Broader whisper post-processing — preventing hallucinated lines from being produced at all (currently we just discard them downstream)
- Multi-voice / laughter audio classification — would let `emotional_spike` be computed honestly for transcript-only clips with real reactions instead of hardcoded
- Tag system overhaul — `shock` tag firing on noise is fixed at the input layer here; deeper tag-pipeline cleanup remains
- Per-game scoring weights — currently per-game configs only adjust detection thresholds and signal weights, not scoring dimension overrides. Could extend if useful.
- More precise hallucination detection — current rule is line-level repeat detection. Deeper checks (token-level entropy, semantic emptiness) would catch subtler hallucination patterns.
