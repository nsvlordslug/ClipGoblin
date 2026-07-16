# ClipGoblin UI Coherence Audit

**Started:** 2026-07-14
**Goal:** Keep ClipGoblin fast to scan and easy to learn as detection, editing, and publishing capabilities grow.

## Product interaction rules

1. Show the next likely action, not every possible action.
2. Keep advanced controls available through progressive disclosure.
3. Allow one active editing/review context at a time.
4. Use one primary action per workflow state; move secondary actions into a menu.
5. Reserve accent colors for selection and primary action. Use semantic colors for success, warning, and failure.
6. Preserve user context when moving between Dashboard, VODs, Clips, Editor, and publishing.

## Prioritized findings

### P0 - Clip Library repeated every feedback control on every card

**Observed:** At a 1280x840 app window, each card exposed three moment ratings, five edit-issue controls, and a note field. Preview and title scanning became secondary to a wall of repeated controls.

**Direction:** Keep cards compact by default. Use one checklist action, allow only one feedback disclosure to open, keep reviewed state visible without reopening the form, and close the disclosure on Escape or when entering selection mode.

**Status:** Implemented and manually validated at the user's normal app window size on 2026-07-14.

### P1 - Settings mixes unrelated jobs in one long page

**Observed:** Connected accounts, BYOK configuration, detection, transcription, storage, templates, appearance, and About share one continuous page. New settings increase search time and make cost-sensitive controls harder to distinguish from ordinary preferences.

**Direction:** Add a compact Settings section navigator with Account, Detection, AI, Editing, Storage, and Appearance. Keep values on one route initially to avoid state churn, but scroll/focus one section at a time. Place cost warnings beside the setting that incurs cost.

**Status:** Implemented, browser-validated at 1280x800 and 1024x720, and manually validated in the native Tauri app by the user on 2026-07-14.

### P1 - VOD cards accumulate state and action controls

**Observed:** A VOD can expose download, open, analyze/reanalyze, view clips, export diagnostics, metadata, delete, progress, cost, and reconnect states. Actions compete even though only one or two are relevant at a given lifecycle stage.

**Direction:** Derive one state-specific primary action (Download, Analyze, View clips, Retry, or Resume). Put secondary and destructive actions in an overflow menu. Keep progress and recovery text adjacent to the primary action.

**Status:** Implemented, code-validated, and manually validated in the native Tauri app by the user on 2026-07-14.

### P1 - Editor combines multiple workflows in one scrolling rail

**Observed:** Trim/crop, subtitle editing, styling, post copy, platform settings, export, and upload can occupy the same right-side scroll. Publishing controls can obscure editing controls and completion feedback.

**Direction:** Separate the rail into Edit, Captions, and Publish tabs while keeping the preview persistent. Preserve unsaved state across tabs. Make Export/Publish a sticky footer only inside Publish.

**Status:** Implemented, code-validated, and manually validated in the native app at the user's normal window size on 2026-07-14.

### P1 - Dashboard and Clip Library have overlapping review roles

**Observed:** Dashboard Workbench and Clip Library both present clip review entry points, but their responsibilities are not explicit.

**Direction:** Treat Dashboard as a small prioritized inbox (needs review, failed, scheduled soon). Treat Clips as the complete searchable library. Dashboard actions should deep-link to the exact clip and task rather than reproduce full library controls.

**Status:** Implemented, code-validated, and manually validated in the native Tauri app by the user on 2026-07-14. The five-tab workbench is now one ordered action inbox; scheduled-soon remains in the adjacent Next Up panel.

### P2 - Dense badges and accent colors flatten hierarchy

**Observed:** Scores, provenance, status, rating, scheduling, and primary actions frequently use saturated violet, pink, blue, green, and amber at the same visual weight.

**Direction:** Keep score and provenance quiet by default. Use violet/pink for active selection or the primary command, semantic colors for lifecycle status, and neutral outlines for metadata.

### P2 - Window-size behavior needs a repeatable visual gate

**Observed:** The app is commonly used around 1280x800, where four-column cards and long control labels have little spare width.

**Direction:** Verify the primary pages at 1024x720, 1280x800, 1440x900, and a maximized desktop window. Check text fit, scroll reachability, dialog containment, and focus order.

## Implementation slices

1. **Clip Library density:** progressive feedback disclosure and compact reviewed state.
2. **Settings navigation:** section navigator and clearer cost/advanced grouping.
3. **VOD action hierarchy:** one lifecycle-aware primary action plus overflow.
4. **Editor workspace:** Edit, Captions, and Publish task tabs.
5. **Dashboard contract:** prioritized inbox with exact deep links.
6. **Visual/accessibility pass:** color hierarchy, keyboard flow, text fit, and multi-size screenshots.

Each slice should ship independently with a manual screenshot comparison and the normal frontend/Rust regression checks.
