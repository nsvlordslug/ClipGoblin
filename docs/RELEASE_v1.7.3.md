ClipGoblin v1.7.3 improves clip preservation, publishing recovery, and editing reliability.

## Fixes
- Reanalysis preserves reviewed or edited clips and their saved exports, schedules, and history.
- Scheduled posts stay bound to the account selected when scheduling. Older schedules without a recorded account must be reviewed and recreated.
- Interrupted YouTube uploads can check and resume the original session instead of blindly starting another upload. Expired or missing sessions require a YouTube Studio check before a new upload.
- Uploading both portrait and landscape formats tracks each format separately.
- Bulk selection follows the visible clips; disabled subtitles stay disabled during automatic export preparation.
- Export completion, undo, and caption generation respect the latest saved edits and publish text.
- Windows download-folder validation and imported-video trim bounds are corrected.
- Unknown preview durations display a placeholder until media loads; montage segments stay within source duration.

## Verification and limits
- 169 frontend tests passed on the final candidate. The preceding native regression suite passed 781 tests, with two existing ignored tests; the final montage change passed its real-FFmpeg regression.
- Two private YouTube tests verified same-session interrupted-upload recovery and a due scheduled upload. Both completed on the intended account. The schedule ran after an intentional pause; this is not a claim of on-time unattended dispatch.
- The production installer was installed and launched through the normal desktop shortcut. All 204 saved clips loaded; playback and the original duration defect were checked. All 18 database tables, including settings and accounts, were unchanged after installation and launch.
- Windows x64 installers retain the portable CPU build settings and were checked for absence of AVX-512 instructions. Updater signatures use the existing trusted key.
- These checks are not a completed penetration test or a new dependency-vulnerability scan. TikTok posting approval and platform restrictions are unchanged.

## Update
Restart ClipGoblin to check for the update, then click **Install & restart**. Manual Windows installers are attached below.

The release uses the exact locally verified installer bytes. Its source snapshot is versioned with this release; CI rebuilding is skipped for this tag to avoid replacing those tested artifacts.
