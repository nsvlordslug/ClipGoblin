//! Scripted transport only: no Google requests, user database, or real credentials.
use super::*;
use std::collections::VecDeque;

const SESSION_URI: &str =
    "https://www.googleapis.com/upload/youtube/v3/videos?upload_id=mock-session-secret";
const VIDEO_ID: &str = "abcdefghijk";
const TOKEN: &str = "fixture-token";

struct Fixture {
    db: DbConn,
    meta: UploadMeta,
    path: std::path::PathBuf,
}
impl Fixture {
    fn new(size: usize) -> Self {
        let conn = Connection::open_in_memory().unwrap();
        db::run_migrations(&conn).unwrap();
        conn.execute_batch("INSERT INTO vods(id,title) VALUES('vod','Fixture VOD');
            INSERT INTO highlights(id,vod_id,start_seconds,end_seconds) VALUES('highlight','vod',0,17);
            INSERT INTO clips(id,highlight_id,vod_id,title,start_seconds,end_seconds)
            VALUES('clip','highlight','vod','Original title',0,17);").unwrap();
        db::save_setting(&conn, "youtube_channel_id", "account-a").unwrap();
        db::save_setting(&conn, "youtube_access_token", TOKEN).unwrap();
        assert!(matches!(
            db::begin_upload_variant(&conn, "clip", "youtube", "9:16", false).unwrap(),
            db::UploadClaim::Acquired
        ));
        let path = std::env::temp_dir().join(format!(
            "clipgoblin-youtube-session-{}.mp4",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, vec![7_u8; size]).unwrap();
        let meta=serde_json::from_value(serde_json::json!({"title":"Original title","description":"Original description",
            "tags":["tag"],"visibility":"private","clip_id":"clip","force":false,"target_account_id":"account-a",
            "artifact_path":path.to_string_lossy(),"artifact_revision":"render-v1","artifact_aspect_ratio":"9:16"})).unwrap();
        Self {
            db: std::sync::Mutex::new(conn),
            meta,
            path,
        }
    }
    fn reload(&self) -> Session {
        let conn = self.db.lock().unwrap();
        // Simulate time passing without wall-clock sleeps or a live service.
        conn.execute("UPDATE youtube_upload_sessions SET retry_at=0", [])
            .unwrap();
        latest(&conn, "clip", "9:16").unwrap().unwrap()
    }
    fn history(&self) -> db::UploadHistoryRow {
        db::get_upload_for_variant(&self.db.lock().unwrap(), "clip", "youtube", "9:16")
            .unwrap()
            .unwrap()
    }
    fn add_schedule(&self, id: &str, status: &str, account: &str, video: Option<&str>) {
        let mut meta = self.meta.clone();
        meta.target_account_id = Some(account.into());
        self.db.lock().unwrap().execute("INSERT INTO scheduled_uploads(id,clip_id,platform,scheduled_time,status,platform_video_id,upload_meta_json,created_at)
            VALUES(?1,'clip','youtube','2099-01-01',?2,?3,?4,'now')",
            params![id,status,video,serde_json::to_string(&meta).unwrap()]).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn response(status: u16, range: Option<&str>, retry: Option<&str>) -> WireResponse {
    WireResponse {
        status,
        location: if status == 200 {
            Some(SESSION_URI.into())
        } else {
            None
        },
        range: range.map(str::to_string),
        retry_after: retry.map(str::to_string),
        body: if matches!(status, 200 | 201) {
            format!("{{\"id\":\"{VIDEO_ID}\"}}").into_bytes()
        } else {
            vec![]
        },
    }
}
fn interrupted() -> Result<WireResponse, AppError> {
    Err(AppError::Api("mock connection lost".into()))
}
struct Script<'a> {
    replies: VecDeque<(Method, Result<WireResponse, AppError>)>,
    sent: Vec<(Method, Option<String>, usize)>,
    switch_after_probe: Option<&'a DbConn>,
}
impl<'a> Script<'a> {
    fn new(replies: Vec<(Method, Result<WireResponse, AppError>)>) -> Self {
        Self {
            replies: replies.into(),
            sent: vec![],
            switch_after_probe: None,
        }
    }
}
#[async_trait(?Send)]
impl Transport for Script<'_> {
    async fn send(&mut self, request: WireRequest<'_>) -> Result<WireResponse, AppError> {
        assert!(
            request.uri
                == if request.method == Method::Init {
                    INIT_URL
                } else {
                    SESSION_URI
                }
        );
        let (method, response) = self.replies.pop_front().expect("unexpected request");
        assert!(method == request.method);
        if method == Method::Probe {
            assert!(request.body.is_empty());
            if let Some(db) = self.switch_after_probe.take() {
                db::save_setting(&db.lock().unwrap(), "youtube_channel_id", "account-b").unwrap();
            }
        }
        self.sent.push((method, request.range, request.body.len()));
        response
    }
}
async fn lost_final(fixture: &Fixture) {
    let mut wire = Script::new(vec![
        (Method::Init, Ok(response(200, None, None))),
        (Method::Chunk, interrupted()),
    ]);
    let error = start_with_transport(
        &fixture.db,
        fixture.path.to_str().unwrap(),
        &fixture.meta,
        TOKEN,
        &mut wire,
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("Upload outcome is uncertain"));
    assert!(!error.to_string().contains("mock-session-secret"));
    assert_eq!(fixture.history().status, "uncertain");
}

#[tokio::test]
async fn lost_final_response_reloads_saved_session_and_completes_without_artifact_or_new_post() {
    let fixture = Fixture::new(17);
    lost_final(&fixture).await;
    let mut session = fixture.reload();
    assert!(session
        .encrypted_uri
        .as_ref()
        .unwrap()
        .starts_with("dpapi:"));
    assert_ne!(session.encrypted_uri.as_deref(), Some(SESSION_URI));
    assert_eq!(session.meta.title, "Original title");
    assert_eq!(session.artifact_revision, "render-v1");
    assert_eq!(session.sha256, format!("{:x}", Sha256::digest(vec![7; 17])));
    std::fs::remove_file(&fixture.path).unwrap();
    let mut wire = Script::new(vec![(Method::Probe, Ok(response(200, None, None)))]);
    let outcome = recover_session(&fixture.db, &mut session, TOKEN, &mut wire)
        .await
        .unwrap();
    assert_eq!(outcome.status, "completed");
    assert_eq!(outcome.original_title.as_deref(), Some("Original title"));
    assert_eq!(wire.sent.len(), 1);
    assert_eq!(
        fixture.history().platform_video_id.as_deref(),
        Some(VIDEO_ID)
    );
    let conn = fixture.db.lock().unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM scheduled_uploads", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        latest(&conn, "clip", "9:16").unwrap().unwrap().state,
        "completed"
    );
    drop(conn);
    assert!(matches!(
        prepare_recovery(&fixture.db, "clip", "9:16").unwrap(),
        RecoveryPlan::Resolved(_)
    ));
}

#[tokio::test]
async fn resumed_bytes_start_at_probed_offset_after_partial_acknowledgement() {
    let fixture = Fixture::new(CHUNK_SIZE + 11);
    let mut first = Script::new(vec![
        (Method::Init, Ok(response(200, None, None))),
        (
            Method::Chunk,
            Ok(response(308, Some("bytes=0-262143"), None)),
        ),
        (Method::Chunk, interrupted()),
    ]);
    assert!(start_with_transport(
        &fixture.db,
        fixture.path.to_str().unwrap(),
        &fixture.meta,
        TOKEN,
        &mut first
    )
    .await
    .is_err());
    let mut session = fixture.reload();
    assert_eq!(session.offset, 262144);
    let mut wire = Script::new(vec![
        (
            Method::Probe,
            Ok(response(308, Some("bytes=0-524287"), None)),
        ),
        (Method::Chunk, Ok(response(201, None, None))),
    ]);
    assert_eq!(
        recover_session(&fixture.db, &mut session, TOKEN, &mut wire)
            .await
            .unwrap()
            .status,
        "completed"
    );
    assert_eq!(
        wire.sent[1].1.as_deref(),
        Some(format!("bytes 524288-{}/{}", CHUNK_SIZE + 10, CHUNK_SIZE + 11).as_str())
    );
    assert_eq!(wire.sent[1].2, CHUNK_SIZE + 11 - 524288);
}

#[tokio::test]
async fn expired_session_requires_single_use_account_bound_absence_review() {
    let fixture = Fixture::new(17);
    lost_final(&fixture).await;
    let mut session = fixture.reload();
    let mut wire = Script::new(vec![(Method::Probe, Ok(response(404, None, None)))]);
    let outcome = recover_session(&fixture.db, &mut session, TOKEN, &mut wire)
        .await
        .unwrap();
    assert_eq!(outcome.status, "review_required");
    assert_eq!(outcome.original_title.as_deref(), Some("Original title"));
    let review = outcome.review_id.unwrap();
    assert!(review_absent(&fixture.db, "clip", "9:16", &review, "account-a", false).is_err());
    assert!(review_absent(&fixture.db, "clip", "9:16", "stale", "account-a", true).is_err());
    db::save_setting(
        &fixture.db.lock().unwrap(),
        "youtube_channel_id",
        "account-b",
    )
    .unwrap();
    assert!(review_absent(&fixture.db, "clip", "9:16", &review, "account-a", true).is_err());
    db::save_setting(
        &fixture.db.lock().unwrap(),
        "youtube_channel_id",
        "account-a",
    )
    .unwrap();
    review_absent(&fixture.db, "clip", "9:16", &review, "account-a", true).unwrap();
    assert_eq!(fixture.history().status, "failed");
    assert!(review_absent(&fixture.db, "clip", "9:16", &review, "account-a", true).is_err());
    let conn = fixture.db.lock().unwrap();
    let retained = latest(&conn, "clip", "9:16").unwrap().unwrap();
    assert_eq!(retained.state, "reviewed_absent");
    assert!(retained.encrypted_uri.is_some());
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM youtube_upload_sessions WHERE reviewed_absent_at IS NOT NULL",
            [],
            |row| row.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert!(matches!(
        db::begin_upload_variant(&conn, "clip", "youtube", "9:16", false).unwrap(),
        db::UploadClaim::Acquired
    ));
}

#[tokio::test]
async fn transient_status_and_retry_after_never_offer_clear_or_send_media() {
    let fixture = Fixture::new(17);
    lost_final(&fixture).await;
    for code in [503, 308] {
        let mut session = fixture.reload();
        let mut wire = Script::new(vec![(Method::Probe, Ok(response(code, None, Some("60"))))]);
        let outcome = recover_session(&fixture.db, &mut session, TOKEN, &mut wire)
            .await
            .unwrap();
        assert_eq!(outcome.status, "retry_later");
        assert!(outcome.review_id.is_none());
        assert_eq!(wire.sent.len(), 1);
        assert!(session.retry_at >= chrono::Utc::now().timestamp() + 58);
        assert_eq!(
            recover_session(&fixture.db, &mut session, TOKEN, &mut wire)
                .await
                .unwrap()
                .status,
            "retry_later"
        );
        assert_eq!(wire.sent.len(), 1);
        assert!(review_absent(&fixture.db, "clip", "9:16", "any", "account-a", true).is_err());
    }
}

#[tokio::test]
async fn changed_artifact_and_account_switch_cannot_send_resumed_bytes() {
    let fixture = Fixture::new(17);
    lost_final(&fixture).await;
    std::fs::write(&fixture.path, vec![8; 17]).unwrap();
    let mut session = fixture.reload();
    let mut wire = Script::new(vec![(Method::Probe, Ok(response(308, None, None)))]);
    assert!(recover_session(&fixture.db, &mut session, TOKEN, &mut wire)
        .await
        .unwrap_err()
        .to_string()
        .contains("changed"));
    assert_eq!(wire.sent.len(), 1);
    assert_eq!(fixture.history().status, "uncertain");
    assert!(review_absent(&fixture.db, "clip", "9:16", "any", "account-a", true).is_err());
    std::fs::write(&fixture.path, vec![7; 17]).unwrap();
    let mut session = fixture.reload();
    let mut wire = Script::new(vec![(Method::Probe, Ok(response(308, None, None)))]);
    wire.switch_after_probe = Some(&fixture.db);
    assert!(recover_session(&fixture.db, &mut session, TOKEN, &mut wire)
        .await
        .is_err());
    assert_eq!(wire.sent.len(), 1);
    let mut before = Script::new(vec![]);
    assert!(
        recover_session(&fixture.db, &mut session, TOKEN, &mut before)
            .await
            .is_err()
    );
    assert!(before.sent.is_empty());
}

#[tokio::test]
async fn completion_updates_only_original_schedule_preserving_future_and_previous_remote_videos() {
    let mut fixture = Fixture::new(17);
    fixture.add_schedule("original", "uploading", "account-a", None);
    fixture.add_schedule("future", "pending", "account-a", None);
    fixture.add_schedule("old", "completed", "account-a", Some("old-video"));
    fixture.add_schedule("other-account", "uploading", "account-b", None);
    fixture.meta.scheduled_upload_id = Some("original".into());
    lost_final(&fixture).await;
    fixture
        .db
        .lock()
        .unwrap()
        .execute(
            "UPDATE scheduled_uploads SET status='failed' WHERE id='original'",
            [],
        )
        .unwrap();
    let mut session = fixture.reload();
    let mut wire = Script::new(vec![(Method::Probe, Ok(response(200, None, None)))]);
    recover_session(&fixture.db, &mut session, TOKEN, &mut wire)
        .await
        .unwrap();
    let conn = fixture.db.lock().unwrap();
    let get = |id: &str| {
        conn.query_row(
            "SELECT status,platform_video_id FROM scheduled_uploads WHERE id=?1",
            [id],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
        )
        .unwrap()
    };
    assert_eq!(get("original"), ("completed".into(), Some(VIDEO_ID.into())));
    assert_eq!(get("future"), ("pending".into(), None));
    assert_eq!(get("old"), ("completed".into(), Some("old-video".into())));
    assert_eq!(get("other-account"), ("uploading".into(), None));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM scheduled_uploads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        4
    );
}

#[tokio::test]
async fn legacy_uncertainty_requires_review_without_claiming_known_original_account() {
    let fixture = Fixture::new(17);
    {
        let conn = fixture.db.lock().unwrap();
        conn.execute(
            "UPDATE upload_history SET status='uncertain',artifact_aspect_ratio=''",
            [],
        )
        .unwrap();
    }
    let mut session = match prepare_recovery(&fixture.db, "clip", "9:16").unwrap() {
        RecoveryPlan::Ready(session) => session,
        _ => panic!("expected review"),
    };
    assert!(session.legacy);
    let mut wire = Script::new(vec![]);
    let outcome = recover_session(&fixture.db, &mut session, TOKEN, &mut wire)
        .await
        .unwrap();
    assert_eq!(outcome.aspect_ratio, "");
    assert_eq!(outcome.status, "review_required");
    assert!(outcome.original_title.is_none());
    assert!(outcome
        .message
        .contains("no saved session or verified original account"));
    assert!(wire.sent.is_empty());
    review_absent(
        &fixture.db,
        "clip",
        "",
        outcome.review_id.as_deref().unwrap(),
        "account-a",
        true,
    )
    .unwrap();
}

#[tokio::test]
async fn disk_restart_migrations_preserve_attempt_identity_and_resolve_lost_final_with_only_probe()
{
    struct RemoveDatabase(std::path::PathBuf);
    impl Drop for RemoveDatabase {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    let fixture = Fixture::new(17);
    lost_final(&fixture).await;
    let original = fixture.reload();
    let database = RemoveDatabase(fixture.path.with_extension("sqlite"));
    {
        let conn = fixture.db.lock().unwrap();
        // Also cover an abrupt stop with the old startup status still 'uploading'.
        conn.execute("UPDATE upload_history SET status='uploading'", [])
            .unwrap();
        conn.execute("VACUUM INTO ?1", [database.0.to_string_lossy().as_ref()])
            .unwrap();
    }
    drop(fixture); // Close the original connection and remove the rendered video.
    let reopened = Connection::open(&database.0).unwrap();
    db::run_migrations(&reopened).unwrap();
    let history = db::get_upload_for_variant(&reopened, "clip", "youtube", "9:16")
        .unwrap()
        .unwrap();
    assert_eq!(history.status, "uncertain");
    assert_eq!(
        history.job_id.as_deref(),
        Some(original.attempt_id.as_str())
    );
    let reopened = std::sync::Mutex::new(reopened);
    let mut session = match prepare_recovery(&reopened, "clip", "9:16").unwrap() {
        RecoveryPlan::Ready(session) => session,
        _ => panic!("expected saved session"),
    };
    assert!(!session.legacy);
    assert_eq!(session.attempt_id, original.attempt_id);
    assert_eq!(session.encrypted_uri, original.encrypted_uri);
    let mut wire = Script::new(vec![(Method::Probe, Ok(response(200, None, None)))]);
    assert_eq!(
        recover_session(&reopened, &mut session, TOKEN, &mut wire)
            .await
            .unwrap()
            .status,
        "completed"
    );
    assert_eq!(wire.sent.len(), 1);
    assert_eq!(
        reopened
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM scheduled_uploads WHERE platform_video_id=?1",
                [VIDEO_ID],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
    drop(reopened);
}

#[tokio::test]
async fn replaced_attempt_and_old_review_cannot_send_or_overwrite_the_current_attempt() {
    let fixture = Fixture::new(17);
    lost_final(&fixture).await;
    let mut old = fixture.reload();
    let mut expired = Script::new(vec![(Method::Probe, Ok(response(410, None, None)))]);
    let old_review = recover_session(&fixture.db, &mut old, TOKEN, &mut expired)
        .await
        .unwrap()
        .review_id
        .unwrap();
    review_absent(&fixture.db, "clip", "9:16", &old_review, "account-a", true).unwrap();
    assert!(matches!(
        db::begin_upload_variant(
            &fixture.db.lock().unwrap(),
            "clip",
            "youtube",
            "9:16",
            false
        )
        .unwrap(),
        db::UploadClaim::Acquired
    ));
    lost_final(&fixture).await;
    let mut current = fixture.reload();
    assert_ne!(current.attempt_id, old.attempt_id);
    let mut expired = Script::new(vec![(Method::Probe, Ok(response(404, None, None)))]);
    let review = recover_session(&fixture.db, &mut current, TOKEN, &mut expired)
        .await
        .unwrap();
    assert_ne!(review.review_id.as_deref(), Some(old_review.as_str()));
    assert!(review_absent(&fixture.db, "clip", "9:16", &old_review, "account-a", true).is_err());
    let mut none = Script::new(vec![]);
    assert!(recover_session(&fixture.db, &mut old, TOKEN, &mut none)
        .await
        .is_err());
    assert!(finish(&fixture.db, &mut old, VIDEO_ID).is_err());
    assert!(none.sent.is_empty());
    assert_eq!(
        fixture.history().job_id.as_deref(),
        Some(current.attempt_id.as_str())
    );
    assert_eq!(fixture.history().status, "uncertain");
}
