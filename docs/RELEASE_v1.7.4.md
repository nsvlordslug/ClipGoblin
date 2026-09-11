# ClipGoblin v1.7.4

ClipGoblin v1.7.4 makes the TikTok publishing flow match TikTok's current Content Posting requirements and clarifies the beta's current platform limits.

## TikTok publishing

- Shows the audience choices returned for the connected TikTok account without choosing one automatically.
- Requires the creator to review the current account, video, caption, audience, interaction settings, commercial-content disclosures, and consent before Direct Post.
- Stops and explains the problem when an audience is unavailable rather than silently substituting another audience.
- Validates the current account's posting status and maximum video duration before upload.
- Prevents the private `Only me` audience from being combined with branded content.
- Keeps TikTok out of Auto-Ship; TikTok posts require a creator-reviewed publishing step.

## Current availability

ClipGoblin remains available only to approved beta testers. Until TikTok approves the Direct Post integration, TikTok limits unaudited posting to private visibility. This release does not claim or bypass wider TikTok posting access.

## Verification

- 169 frontend tests passed.
- 784 native tests passed; two existing tests remain intentionally ignored.
- TypeScript, Vite production, Cargo, Tauri installer, and updater-signature checks passed.
