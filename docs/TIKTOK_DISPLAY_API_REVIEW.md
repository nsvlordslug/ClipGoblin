# TikTok Display API review package

This package is for the revision that adds the Display API `video.list` scope.
Do not release the matching desktop build until TikTok approves that scope.

## App review description

ClipGoblin is a Windows desktop app for gaming creators. Login Kit opens TikTok OAuth 2.0 with PKCE. `user.info.basic` verifies the connected account. `video.list` is used only when the user clicks Refresh stats in Analytics to query view and like counts for that user's public TikTok videos previously published through ClipGoblin. Results stay in the user's local database and are shown only to that user. `video.publish` directly posts a rendered clip after the user selects a TikTok-provided audience, interaction controls, disclosures, and music consent. `video.upload` sends the rendered clip to TikTok Inbox as a draft. The demo shows consent for all scopes, verified identity, a direct post, a draft upload, and Analytics refreshing a previously published public video.

## Demo recording checklist

Record one continuous video in the real ClipGoblin desktop app:

1. Start disconnected from TikTok, then click Connect TikTok.
2. Show TikTok's consent screen with `user.info.basic`, `video.publish`, `video.upload`, and `video.list`.
3. Complete consent and show the verified TikTok account in ClipGoblin.
4. Open Publish and briefly show the existing approved Direct Post and draft choices, TikTok-provided audience and interaction controls, commercial-content disclosures, and music consent.
5. Open Analytics and show the existing public TikTok upload before refreshing.
6. Click Refresh stats and show that video's view and like counts populate.
7. State that ClipGoblin requests metrics only when the user clicks Refresh stats, reads only the connected creator's public videos, and stores the result locally.

Before upload, inspect the final recording from start to finish and confirm every selected product and scope is visibly demonstrated.

## Approval and release sequence

1. Wait for the current Direct Post revision to leave TikTok's locked review state.
2. Add Display API and `video.list` to a new production revision.
3. Publish the updated privacy page.
4. Upload the inspected demo and paste the review description.
5. Submit the revision.
6. Keep the current production build live while TikTok reviews the new scope.
7. After approval, ship the desktop build and ask testers to reconnect TikTok once.
