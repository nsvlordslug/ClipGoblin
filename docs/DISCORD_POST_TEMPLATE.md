# Discord Transparency Post Template

How to format ClipGoblin release announcements ("transparency posts") for the tester
Discord. Used by any agent (Codex, Claude) turning release notes into a post.
Spacing and formatting are strict — follow the SHAPE EXAMPLE at the bottom exactly.

## Structure (in order)

1. **Header:** `**ClipGoblin vX.Y.Z is live** 👺<one theme emoji>`
   - Bold. Always 👺; the second emoji matches the update's focus
     (🛠️ hardening, 🚀 big feature drop, 💬 subtitles/captions, 🎬 editing).
2. One blank line, then a 1–2 sentence framing paragraph sized honestly
   ("A focused one — ..." / "The biggest update yet — ..."), ending with:
   "It updates automatically next time you open the app (restart it if it's already running)."
3. **Sections:**
   - Header line: `**<emoji> Section name**` (emoji inside the bold, at the start).
   - Bullets start on the very next line — no blank line between header and first bullet.
   - Bullets use `- ` (hyphen space), never `*`. No blank lines between bullets.
   - Exactly one blank line between sections.
4. Always end with these two sections when applicable:
   - `**📝 Transparency notes**` — honest fine print: licensing, caps, compatibility
     caveats, and reassurances like "No changes to AI models, pricing, or BYOK billing
     in this one."
   - `**✅ Verified before shipping**` — 1–3 bullets condensing tests/build/installer checks.
5. **Closer** (one blank line after the last section):
   - One line asking testers to keep reporting the things this update touched, ending
     with: "The in-app bug reporter drops them straight into our tracker."
   - Blank line, then: `**Download / update:** https://github.com/nsvlordslug/ClipGoblin/releases/tag/vX.Y.Z`
   - Blank line, then: `Thanks for testing 🙏`

## Voice & content rules

- Tester-facing plain language — what users will notice, never internal names
  (functions, files, structs).
- Honest, zero hype. Reassure rather than oversell (e.g. "that's not a 75% bigger
  bill, but more selected clips can mean more paid title/caption calls").
- NEVER use the word "free" for the app or a mode. Say "local", "no-AI-key",
  "no AI cost", or "no subscription cost" instead.
- Tighten wordy bullets; merge overlapping ones.

## Length

Discord's limit is 2000 characters per message. If the post exceeds ~1900 characters,
split into "Message 1" and "Message 2" at a section boundary, roughly balanced.

## Output

Return ONLY the finished post as raw Discord markdown inside a copyable code block
(two blocks if split), so it can be pasted without losing formatting. No commentary
outside the block(s). Before posting, confirm the release tag actually exists on
GitHub so the download link doesn't 404.

## Shape example — copy this blank-line placement exactly

**ClipGoblin v9.9.9 is live** 👺🛠️

A focused one — <what it improves>. It updates automatically next time you open the app (restart it if it's already running).

**💬 Section name**
- First bullet.
- Second bullet.

**📝 Transparency notes**
- Honest caveat.
- No changes to AI models, pricing, or BYOK billing in this one.

**✅ Verified before shipping**
- Passed <checks and test counts>.

Keep sending clips where <update-relevant things> feel off — real examples are what move it forward. The in-app bug reporter drops them straight into our tracker.

**Download / update:** https://github.com/nsvlordslug/ClipGoblin/releases/tag/v9.9.9

Thanks for testing 🙏
