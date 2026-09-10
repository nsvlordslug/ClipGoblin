//! Durable YouTube resumable sessions. Session URLs are credentials: encrypt at
//! rest, never serialize them to the frontend, and never put them in errors.

use super::UploadMeta;
use crate::{crypto, db, error::AppError, DbConn};
use async_trait::async_trait;
use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use std::io::SeekFrom;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

const INIT_URL: &str =
    "https://www.googleapis.com/upload/youtube/v3/videos?uploadType=resumable&part=snippet,status";
const CHUNK_SIZE: usize = 5 * 1024 * 1024;
const MAX_RESPONSE: usize = 64 * 1024;
const MAX_REQUESTS: usize = 1024;

#[cfg(test)]
#[path = "youtube_session_protocol_tests.rs"]
mod protocol_tests;
#[cfg(all(test, windows))]
#[path = "youtube_session_tests.rs"]
mod tests;

#[derive(Debug, serde::Serialize)]
pub struct RecoveryResult {
    pub status: String,
    pub video_url: Option<String>,
    pub message: String,
    pub review_id: Option<String>,
    pub account_id: Option<String>,
    pub aspect_ratio: String,
    pub original_title: Option<String>,
}

// Deliberately no Debug/Serialize: encrypted_uri and metadata stay inside Rust.
struct Session {
    attempt_id: String,
    clip_id: String,
    aspect_ratio: String,
    account_id: String,
    encrypted_uri: Option<String>,
    artifact_path: String,
    artifact_revision: String,
    size: u64,
    sha256: String,
    meta: UploadMeta,
    offset: u64,
    state: String,
    video_id: Option<String>,
    review_id: Option<String>,
    retry_at: i64,
    legacy: bool,
    history_id: String,
}

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS youtube_upload_sessions (
        attempt_id TEXT PRIMARY KEY, clip_id TEXT NOT NULL, aspect_ratio TEXT NOT NULL,
        account_id TEXT NOT NULL, encrypted_uri TEXT, artifact_path TEXT NOT NULL,
        artifact_revision TEXT NOT NULL, size INTEGER NOT NULL, sha256 TEXT NOT NULL,
        meta_json TEXT NOT NULL, acknowledged_offset INTEGER NOT NULL DEFAULT 0,
        state TEXT NOT NULL, video_id TEXT, review_id TEXT, retry_at INTEGER NOT NULL DEFAULT 0,
        legacy INTEGER NOT NULL DEFAULT 0, history_id TEXT NOT NULL,
        created_at TEXT NOT NULL, updated_at TEXT NOT NULL, reviewed_absent_at TEXT
    );
    CREATE INDEX IF NOT EXISTS youtube_sessions_clip_format ON youtube_upload_sessions(clip_id, aspect_ratio);")
}

fn lock_db(db: &DbConn) -> Result<std::sync::MutexGuard<'_, Connection>, AppError> {
    db.lock()
        .map_err(|_| AppError::Database("Could not access upload recovery records.".into()))
}

fn session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    let meta_json: String = row.get(9)?;
    let meta = serde_json::from_str(&meta_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(Session {
        attempt_id: row.get(0)?,
        clip_id: row.get(1)?,
        aspect_ratio: row.get(2)?,
        account_id: row.get(3)?,
        encrypted_uri: row.get(4)?,
        artifact_path: row.get(5)?,
        artifact_revision: row.get(6)?,
        size: row.get(7)?,
        sha256: row.get(8)?,
        meta,
        offset: row.get(10)?,
        state: row.get(11)?,
        video_id: row.get(12)?,
        review_id: row.get(13)?,
        retry_at: row.get(14)?,
        legacy: row.get(15)?,
        history_id: row.get(16)?,
    })
}

fn latest(
    conn: &Connection,
    clip_id: &str,
    aspect_ratio: &str,
) -> Result<Option<Session>, AppError> {
    Ok(conn.query_row("SELECT attempt_id,clip_id,aspect_ratio,account_id,encrypted_uri,
        artifact_path,artifact_revision,size,sha256,meta_json,acknowledged_offset,state,
        video_id,review_id,retry_at,legacy,history_id
        FROM youtube_upload_sessions WHERE clip_id=?1 AND aspect_ratio=?2 ORDER BY rowid DESC LIMIT 1",
        params![clip_id, aspect_ratio], session_row).optional()?)
}

fn check_fence(conn: &Connection, session: &Session) -> Result<(), AppError> {
    let latest_id: Option<String> = conn
        .query_row(
            "SELECT attempt_id FROM youtube_upload_sessions
        WHERE clip_id=?1 AND aspect_ratio=?2 ORDER BY rowid DESC LIMIT 1",
            params![session.clip_id, session.aspect_ratio],
            |row| row.get(0),
        )
        .optional()?;
    let history =
        db::get_upload_for_variant(conn, &session.clip_id, "youtube", &session.aspect_ratio)?;
    if latest_id.as_deref() != Some(session.attempt_id.as_str())
        || !history.is_some_and(|row| {
            row.id == session.history_id
                && row.job_id.as_deref() == Some(session.attempt_id.as_str())
                && row.status != "completed"
        })
    {
        return Err(AppError::Api(
            "This recovery record was replaced or already resolved. Refresh its status.".into(),
        ));
    }
    Ok(())
}

fn transaction<T>(
    conn: &Connection,
    action: impl FnOnce() -> Result<T, AppError>,
) -> Result<T, AppError> {
    conn.execute_batch("SAVEPOINT youtube_session_change")?;
    match action() {
        Ok(value) => {
            conn.execute_batch("RELEASE youtube_session_change")?;
            Ok(value)
        }
        Err(error) => {
            let _ = conn.execute_batch(
                "ROLLBACK TO youtube_session_change; RELEASE youtube_session_change",
            );
            Err(error)
        }
    }
}

fn persist_attempt(conn: &Connection, session: &Session) -> Result<(), AppError> {
    transaction(conn, || {
        let changed = conn.execute(
            "UPDATE upload_history SET job_id=?1
            WHERE id=?2 AND clip_id=?3 AND platform='youtube' AND artifact_aspect_ratio=?4
                AND status IN ('uploading','uncertain')",
            params![
                session.attempt_id,
                session.history_id,
                session.clip_id,
                session.aspect_ratio
            ],
        )?;
        if changed != 1 {
            return Err(AppError::Api(
                "Upload claim changed before recovery was saved.".into(),
            ));
        }
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute("INSERT INTO youtube_upload_sessions
            (attempt_id,clip_id,aspect_ratio,account_id,encrypted_uri,artifact_path,artifact_revision,
             size,sha256,meta_json,state,legacy,history_id,created_at,updated_at)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?14)",
            params![session.attempt_id,session.clip_id,session.aspect_ratio,session.account_id,
                session.encrypted_uri,session.artifact_path,session.artifact_revision,session.size,
                session.sha256,serde_json::to_string(&session.meta)?,session.state,session.legacy,session.history_id,now])?;
        Ok(())
    })
}

fn validate_session_uri(uri: &str) -> Result<(), AppError> {
    let parsed = reqwest::Url::parse(uri)
        .map_err(|_| AppError::Api("Invalid YouTube upload session address.".into()))?;
    if uri.len() > 8192
        || parsed.scheme() != "https"
        || parsed.host_str() != Some("www.googleapis.com")
        || parsed.path() != "/upload/youtube/v3/videos"
        || parsed.port_or_known_default() != Some(443)
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(AppError::Api(
            "Rejected an unexpected YouTube upload session address.".into(),
        ));
    }
    Ok(())
}

fn session_uri(session: &Session) -> Result<String, AppError> {
    let encrypted = session
        .encrypted_uri
        .as_deref()
        .ok_or_else(|| AppError::Api("No saved upload session.".into()))?;
    if !encrypted.starts_with("dpapi:") {
        return Err(AppError::Api(
            "The saved upload session is not securely stored.".into(),
        ));
    }
    let uri = crypto::decrypt_sensitive(encrypted).map_err(|_| {
        AppError::Api("The saved upload session cannot be unlocked by this Windows account.".into())
    })?;
    validate_session_uri(&uri)?;
    Ok(uri)
}

struct Artifact {
    file: tokio::fs::File,
    path: String,
    size: u64,
    sha256: String,
}

async fn open_artifact(path: &str) -> Result<Artifact, AppError> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1); // FILE_SHARE_READ: no replacement or modification during hash/transfer.
    }
    let file = options.open(path).map_err(|_| {
        AppError::Api(
            "The original rendered video is missing or busy. Restore it before resuming.".into(),
        )
    })?;
    let canonical = std::fs::canonicalize(path)
        .map_err(|_| AppError::Api("Cannot resolve the original rendered video.".into()))?;
    let mut file = tokio::fs::File::from_std(file);
    let size = file
        .metadata()
        .await
        .map_err(|_| AppError::Api("Cannot inspect the rendered video.".into()))?
        .len();
    if size == 0 || size > i64::MAX as u64 {
        return Err(AppError::Api(
            "The rendered video has an invalid size.".into(),
        ));
    }
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    let mut read = 0_u64;
    loop {
        let count = file
            .read(&mut buffer)
            .await
            .map_err(|_| AppError::Api("Cannot read the original rendered video.".into()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        read += count as u64;
    }
    if read != size {
        return Err(AppError::Api(
            "The rendered video changed while it was being verified.".into(),
        ));
    }
    file.seek(SeekFrom::Start(0))
        .await
        .map_err(|_| AppError::Api("Cannot seek in the rendered video.".into()))?;
    Ok(Artifact {
        file,
        path: canonical.to_string_lossy().into_owned(),
        size,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Init,
    Probe,
    Chunk,
}
struct WireRequest<'a> {
    method: Method,
    uri: &'a str,
    token: &'a str,
    range: Option<String>,
    body: Vec<u8>,
    total: u64,
}
struct WireResponse {
    status: u16,
    location: Option<String>,
    range: Option<String>,
    retry_after: Option<String>,
    body: Vec<u8>,
}

#[async_trait(?Send)]
trait Transport {
    async fn send(&mut self, request: WireRequest<'_>) -> Result<WireResponse, AppError>;
}

struct GoogleTransport {
    client: reqwest::Client,
}
impl GoogleTransport {
    fn new() -> Result<Self, AppError> {
        Ok(Self {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(std::time::Duration::from_secs(15))
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .map_err(|_| AppError::Api("Cannot initialize the YouTube connection.".into()))?,
        })
    }
}

#[async_trait(?Send)]
impl Transport for GoogleTransport {
    async fn send(&mut self, request: WireRequest<'_>) -> Result<WireResponse, AppError> {
        validate_session_uri(request.uri)?;
        let is_init = request.method == Method::Init;
        let mut builder = self
            .client
            .request(
                if is_init {
                    reqwest::Method::POST
                } else {
                    reqwest::Method::PUT
                },
                request.uri,
            )
            .bearer_auth(request.token)
            .header("Content-Length", request.body.len().to_string())
            .header(
                "Content-Type",
                if is_init {
                    "application/json; charset=UTF-8"
                } else {
                    "video/mp4"
                },
            );
        if is_init {
            builder = builder
                .header("X-Upload-Content-Length", request.total)
                .header("X-Upload-Content-Type", "video/mp4");
        }
        if let Some(range) = request.range {
            builder = builder.header("Content-Range", range);
        }
        let mut response = builder.body(request.body).send().await.map_err(|_| {
            AppError::Api(
                "YouTube connection was interrupted. The saved session can be checked again."
                    .into(),
            )
        })?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .filter(|value| value.len() <= 8192)
                .map(str::to_string)
        };
        let mut result = WireResponse {
            status: response.status().as_u16(),
            location: header("location"),
            range: header("range"),
            retry_after: header("retry-after"),
            body: Vec::new(),
        };
        while let Some(chunk) = response.chunk().await.map_err(|_| {
            AppError::Api("YouTube response was interrupted. Check the saved session again.".into())
        })? {
            if result.body.len() + chunk.len() > MAX_RESPONSE {
                return Err(AppError::Api(
                    "YouTube returned an oversized response. Check the saved session again.".into(),
                ));
            }
            result.body.extend_from_slice(&chunk);
        }
        Ok(result)
    }
}

fn video_id(response: &WireResponse) -> Result<String, AppError> {
    let body: serde_json::Value = serde_json::from_slice(&response.body).map_err(|_| {
        AppError::Api(
            "YouTube completion response was invalid. Check the saved session again.".into(),
        )
    })?;
    let id = body["id"]
        .as_str()
        .filter(|id| {
            id.len() == 11
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
        .ok_or_else(|| {
            AppError::Api(
                "YouTube did not confirm a valid video ID. Check the saved session again.".into(),
            )
        })?;
    Ok(id.to_string())
}

fn acknowledged_offset(
    range: Option<&str>,
    probe: bool,
    previous: u64,
    sent_end: u64,
    size: u64,
) -> Result<u64, AppError> {
    let invalid = || {
        AppError::Api("YouTube returned an invalid or non-progressing upload range. No additional bytes were sent.".into())
    };
    let next = match range {
        None if probe => 0,
        Some(value) => value
            .trim()
            .strip_prefix("bytes=0-")
            .and_then(|end| {
                if !end.is_empty() && end.bytes().all(|byte| byte.is_ascii_digit()) {
                    end.parse::<u64>().ok()
                } else {
                    None
                }
            })
            .and_then(|end| end.checked_add(1))
            .ok_or_else(invalid)?,
        None => return Err(invalid()),
    };
    if next < previous || next > size || (!probe && (next <= previous || next > sent_end)) {
        return Err(invalid());
    }
    Ok(next)
}

fn retry_deadline(header: Option<&str>, now: i64, fallback: i64) -> i64 {
    match header {
        Some(value) => value
            .trim()
            .parse::<u64>()
            .ok()
            .map(|seconds| now.saturating_add(seconds.min(i64::MAX as u64) as i64))
            .or_else(|| {
                chrono::DateTime::parse_from_rfc2822(value.trim())
                    .ok()
                    .map(|date| date.timestamp())
            })
            .unwrap_or_else(|| now.saturating_add(fallback))
            .max(now),
        None => now.saturating_add(fallback),
    }
}

fn result(session: &Session, status: &str, message: &str) -> RecoveryResult {
    RecoveryResult {
        status: status.into(),
        video_url: session
            .video_id
            .as_ref()
            .map(|id| format!("https://youtu.be/{id}")),
        message: message.into(),
        review_id: session.review_id.clone(),
        account_id: Some(session.account_id.clone()),
        aspect_ratio: session.aspect_ratio.clone(),
        original_title: (!session.legacy && !session.meta.title.is_empty())
            .then(|| session.meta.title.clone()),
    }
}

fn require_current(db: &DbConn, session: &Session, token: &str) -> Result<(), AppError> {
    let conn = lock_db(db)?;
    check_fence(&conn, session)?;
    db::validate_upload_destination(&conn, "youtube", Some(&session.account_id))
        .map_err(AppError::Api)?;
    if db::get_setting(&conn, "youtube_access_token")?.as_deref() != Some(token) {
        return Err(AppError::Api(
            "YouTube connection changed. Reconnect the original account and check recovery again."
                .into(),
        ));
    }
    Ok(())
}

fn save_progress(db: &DbConn, session: &Session) -> Result<(), AppError> {
    let conn = lock_db(db)?;
    check_fence(&conn, session)?;
    conn.execute(
        "UPDATE youtube_upload_sessions SET acknowledged_offset=?2,retry_at=?3,updated_at=?4
        WHERE attempt_id=?1",
        params![
            session.attempt_id,
            session.offset,
            session.retry_at,
            chrono::Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

fn retry_later(
    db: &DbConn,
    session: &mut Session,
    header: Option<&str>,
    message: &str,
) -> Result<RecoveryResult, AppError> {
    session.retry_at = retry_deadline(header, chrono::Utc::now().timestamp(), 5);
    save_progress(db, session)?;
    Ok(result(session, "retry_later", message))
}

fn require_review(db: &DbConn, session: &mut Session) -> Result<RecoveryResult, AppError> {
    let conn = lock_db(db)?;
    check_fence(&conn, session)?;
    session.review_id = Some(uuid::Uuid::new_v4().to_string());
    session.state = "review_required".into();
    conn.execute("UPDATE youtube_upload_sessions SET state='review_required',review_id=?2,updated_at=?3 WHERE attempt_id=?1",
        params![session.attempt_id,session.review_id,chrono::Utc::now().to_rfc3339()])?;
    Ok(result(
        session,
        "review_required",
        if session.legacy {
            "This older upload has no saved session or verified original account. Check YouTube Studio in the currently connected account and any account previously used. Confirm absence only after checking; no new upload has been sent."
        } else if session.encrypted_uri.is_none() {
            "Upload preparation was interrupted before a session could be saved. Check YouTube Studio in the original account and confirm this video is absent before permitting a fresh upload."
        } else {
            "The saved YouTube session expired. Check YouTube Studio in the original account. Confirm that this video is absent before permitting a fresh upload."
        },
    ))
}

fn finish(db: &DbConn, session: &mut Session, id: &str) -> Result<RecoveryResult, AppError> {
    let conn = lock_db(db)?;
    let url = format!("https://youtu.be/{id}");
    transaction(&conn, || {
        check_fence(&conn, session)?;
        let now = chrono::Utc::now().to_rfc3339();
        let changed = conn.execute(
            "UPDATE upload_history SET status='completed',video_url=?1,platform_video_id=?2,
            job_id=?2,uploaded_at=?3,updated_at=?3,last_error=NULL WHERE id=?4 AND job_id=?5",
            params![url, id, now, session.history_id, session.attempt_id],
        )?;
        if changed != 1 {
            return Err(AppError::Api(
                "The upload attempt changed while completion was being recorded.".into(),
            ));
        }
        conn.execute(
            "UPDATE youtube_upload_sessions SET state='completed',video_id=?2,
            acknowledged_offset=size,review_id=NULL,retry_at=0,updated_at=?3 WHERE attempt_id=?1",
            params![session.attempt_id, id, now],
        )?;
        // Match the original reviewed account and artifact. Never rewrite another
        // account's ledger row or replace the other format's remote video ID.
        let matched = conn.execute(
            "UPDATE scheduled_uploads SET status='completed',video_url=?1,
            platform_video_id=?2,job_id=?2,error_message=NULL,stats_updated_at=NULL
            WHERE clip_id=?3 AND platform='youtube' AND json_valid(upload_meta_json)
              AND id=?8
              AND json_extract(upload_meta_json,'$.target_account_id')=?4
              AND COALESCE(json_extract(upload_meta_json,'$.artifact_aspect_ratio'),'')=?5
              AND COALESCE(json_extract(upload_meta_json,'$.artifact_revision'),'')=?6
              AND COALESCE(json_extract(upload_meta_json,'$.artifact_path'),'')=?7
              AND (status IN ('uploading','failed','pending') OR
                   (status='completed' AND platform_video_id=?2))",
            params![
                url,
                id,
                session.clip_id,
                session.account_id,
                session.aspect_ratio,
                session.meta.artifact_revision.as_deref().unwrap_or(""),
                session.meta.artifact_path.as_deref().unwrap_or(""),
                session.meta.scheduled_upload_id
            ],
        )?;
        if matched == 0 {
            conn.execute("INSERT INTO scheduled_uploads (id,clip_id,platform,scheduled_time,status,video_url,
                job_id,platform_video_id,upload_meta_json,created_at)
                VALUES (?1,?2,'youtube',?3,'completed',?4,?5,?5,?6,?3)",
                params![uuid::Uuid::new_v4().to_string(),session.clip_id,now,url,id,serde_json::to_string(&session.meta)?])?;
        }
        Ok(())
    })?;
    session.state = "completed".into();
    session.video_id = Some(id.into());
    session.review_id = None;
    Ok(result(session, "completed", "YouTube confirmed this upload. Its existing video has been recorded; no duplicate was created."))
}

async fn transfer(
    db: &DbConn,
    session: &mut Session,
    token: &str,
    artifact: &mut Artifact,
    transport: &mut dyn Transport,
) -> Result<RecoveryResult, AppError> {
    let uri = session_uri(session)?;
    let mut requests = 0;
    while session.offset < session.size && requests < MAX_REQUESTS {
        require_current(db, session, token)?;
        let start = session.offset;
        let length = (session.size - start).min(CHUNK_SIZE as u64) as usize;
        artifact
            .file
            .seek(SeekFrom::Start(start))
            .await
            .map_err(|_| {
                AppError::Api(
                    "Cannot seek in the original video. The saved session is unchanged.".into(),
                )
            })?;
        let mut body = vec![0; length];
        artifact.file.read_exact(&mut body).await.map_err(|_| {
            AppError::Api("Cannot read the original video. The saved session is unchanged.".into())
        })?;
        // Recheck after file I/O as well as before every request.
        require_current(db, session, token)?;
        {
            let conn = lock_db(db)?;
            check_fence(&conn, session)?;
            db::mark_upload_variant_uncertain(
                &conn,
                &session.clip_id,
                "youtube",
                &session.aspect_ratio,
            )?;
        }
        let response = match transport.send(WireRequest { method: Method::Chunk, uri: &uri, token,
            range: Some(format!("bytes {}-{}/{}",start,start+length as u64-1,session.size)),body,total:session.size }).await {
            Ok(response) => response,
            Err(_) => return retry_later(db, session, None, "YouTube's response was interrupted. Check recovery again to query this same session before sending more bytes."),
        };
        requests += 1;
        match response.status {
            200 | 201 => return finish(db, session, &video_id(&response)?),
            308 => {
                session.offset = acknowledged_offset(response.range.as_deref(),false,start,start+length as u64,session.size)?;
                session.retry_at = retry_deadline(response.retry_after.as_deref(),chrono::Utc::now().timestamp(),0);
                save_progress(db, session)?;
                if session.retry_at > chrono::Utc::now().timestamp() {
                    return Ok(result(session,"retry_later","YouTube asked this upload to wait. Check recovery after the requested delay."));
                }
            }
            404 | 410 => return require_review(db, session),
            _ => return retry_later(db,session,response.retry_after.as_deref(),
                "YouTube has not confirmed completion. The saved session is retained; check recovery again later."),
        }
    }
    retry_later(db,session,None,"YouTube has not confirmed a video ID yet. Check recovery again to query the saved session.")
}

pub async fn start_upload(
    db: &DbConn,
    file_path: &str,
    meta: &UploadMeta,
    token: &str,
) -> Result<(String, String), AppError> {
    let mut transport = GoogleTransport::new()?;
    start_with_transport(db, file_path, meta, token, &mut transport).await
}

async fn start_with_transport(
    db: &DbConn,
    file_path: &str,
    meta: &UploadMeta,
    token: &str,
    transport: &mut dyn Transport,
) -> Result<(String, String), AppError> {
    let variant = db::upload_variant("youtube", meta.artifact_aspect_ratio.as_deref());
    {
        let conn = lock_db(db)?;
        if latest(&conn, &meta.clip_id, variant)?
            .is_some_and(|session| session.retry_at > chrono::Utc::now().timestamp())
        {
            return Err(AppError::Api(
                "YouTube requested a delay. Wait before trying this upload again.".into(),
            ));
        }
    }
    let mut artifact = open_artifact(file_path).await?;
    let mut session = {
        let conn = lock_db(db)?;
        db::validate_upload_destination(&conn, "youtube", meta.target_account_id.as_deref())
            .map_err(AppError::Api)?;
        let history = db::get_upload_for_variant(&conn, &meta.clip_id, "youtube", variant)?
            .filter(|history| history.status == "uploading")
            .ok_or_else(|| AppError::Api("No active upload claim is available.".into()))?;
        let session = Session {
            attempt_id: uuid::Uuid::new_v4().to_string(),
            clip_id: meta.clip_id.clone(),
            aspect_ratio: variant.into(),
            account_id: meta.target_account_id.clone().unwrap_or_default(),
            encrypted_uri: None,
            artifact_path: artifact.path.clone(),
            artifact_revision: meta
                .artifact_revision
                .clone()
                .unwrap_or_else(|| format!("sha256:{}", artifact.sha256)),
            size: artifact.size,
            sha256: artifact.sha256.clone(),
            meta: meta.clone(),
            offset: 0,
            state: "initializing".into(),
            video_id: None,
            review_id: None,
            retry_at: 0,
            legacy: false,
            history_id: history.id,
        };
        persist_attempt(&conn, &session)?;
        session
    };
    require_current(db, &session, token)?;
    let visibility = match meta.visibility.as_str() {
        "public" | "unlisted" => meta.visibility.as_str(),
        _ => "private",
    };
    let body = serde_json::to_vec(&serde_json::json!({
        "snippet":{"title":meta.title,"description":meta.description,"tags":meta.tags,"categoryId":"20"},
        "status":{"privacyStatus":visibility,"selfDeclaredMadeForKids":false}
    }))?;
    let response = transport
        .send(WireRequest {
            method: Method::Init,
            uri: INIT_URL,
            token,
            range: None,
            body,
            total: session.size,
        })
        .await?;
    if !matches!(response.status, 200 | 201) {
        session.retry_at = retry_deadline(
            response.retry_after.as_deref(),
            chrono::Utc::now().timestamp(),
            5,
        );
        save_progress(db, &session)?;
        return Err(AppError::Api(format!("YouTube rejected upload preparation (HTTP {}). No video bytes were sent; wait before retrying.",response.status)));
    }
    let uri = response.location.as_deref().ok_or_else(|| {
        AppError::Api(
            "YouTube did not return a resumable session. No video bytes were sent.".into(),
        )
    })?;
    validate_session_uri(uri)?;
    session.encrypted_uri = Some(crypto::encrypt_sensitive(uri).map_err(|_| {
        AppError::Api("Cannot securely save the YouTube session. No video bytes were sent.".into())
    })?);
    session.state = "active".into();
    {
        let conn = lock_db(db)?;
        check_fence(&conn, &session)?;
        conn.execute("UPDATE youtube_upload_sessions SET encrypted_uri=?2,state='active',updated_at=?3 WHERE attempt_id=?1",
            params![session.attempt_id,session.encrypted_uri,chrono::Utc::now().to_rfc3339()])?;
    }
    let outcome = transfer(db, &mut session, token, &mut artifact, transport).await?;
    if outcome.status == "completed" {
        let id = session
            .video_id
            .ok_or_else(|| AppError::Api("Missing confirmed YouTube video identity.".into()))?;
        Ok((id.clone(), format!("https://youtu.be/{id}")))
    } else {
        Err(AppError::Api(format!(
            "Upload outcome is uncertain. {} Use Check / Resume in the editor.",
            outcome.message
        )))
    }
}

enum RecoveryPlan {
    Ready(Session),
    Resolved(RecoveryResult),
}

fn prepare_recovery(
    db: &DbConn,
    clip_id: &str,
    aspect_ratio: &str,
) -> Result<RecoveryPlan, AppError> {
    if !matches!(aspect_ratio, "" | "9:16" | "16:9") {
        return Err(AppError::Api(
            "Choose a valid YouTube upload format.".into(),
        ));
    }
    let conn = lock_db(db)?;
    let exact = db::get_upload_for_variant(&conn, clip_id, "youtube", aspect_ratio)?;
    let legacy = if aspect_ratio.is_empty() {
        None
    } else {
        db::get_upload_for_clip(&conn, clip_id, "youtube")?
    };
    let history = legacy
        .filter(|row| row.status == "uncertain")
        .or(exact)
        .ok_or_else(|| {
            AppError::Api("No YouTube upload recovery record exists for this format.".into())
        })?;
    let account = db::connected_upload_account(&conn, "youtube")?.ok_or_else(|| {
        AppError::Api("Connect the original YouTube account before checking this upload.".into())
    })?;
    if history.status == "completed" {
        let original = latest(&conn, clip_id, &history.artifact_aspect_ratio)?.filter(|session| {
            session.state == "completed" && session.video_id == history.platform_video_id
        });
        return Ok(RecoveryPlan::Resolved(RecoveryResult {
            status: "completed".into(),
            video_url: history.video_url,
            message: "This upload is already confirmed.".into(),
            review_id: None,
            account_id: original.as_ref().map(|session| session.account_id.clone()),
            aspect_ratio: history.artifact_aspect_ratio,
            original_title: original
                .filter(|session| !session.legacy && !session.meta.title.is_empty())
                .map(|session| session.meta.title),
        }));
    }
    if !matches!(history.status.as_str(), "uncertain" | "uploading") {
        return Err(AppError::Api(
            "This upload does not need recovery. Start a normal upload when ready.".into(),
        ));
    }
    if let Some(session) = latest(&conn, clip_id, &history.artifact_aspect_ratio)? {
        if history.job_id.as_deref() == Some(session.attempt_id.as_str()) {
            db::validate_upload_destination(&conn, "youtube", Some(&session.account_id))
                .map_err(AppError::Api)?;
            return Ok(RecoveryPlan::Ready(session));
        }
    }
    let meta: UploadMeta =
        serde_json::from_value(serde_json::json!({"title":"","description":"","tags":[],
        "visibility":"private","clip_id":clip_id,"force":false,"target_account_id":account,
        "artifact_aspect_ratio":history.artifact_aspect_ratio}))?;
    let session = Session {
        attempt_id: uuid::Uuid::new_v4().to_string(),
        clip_id: clip_id.into(),
        aspect_ratio: history.artifact_aspect_ratio,
        account_id: account,
        encrypted_uri: None,
        artifact_path: String::new(),
        artifact_revision: String::new(),
        size: 0,
        sha256: String::new(),
        meta,
        offset: 0,
        state: "legacy".into(),
        video_id: None,
        review_id: None,
        retry_at: 0,
        legacy: true,
        history_id: history.id,
    };
    persist_attempt(&conn, &session)?;
    Ok(RecoveryPlan::Ready(session))
}

pub async fn recover(
    db: &DbConn,
    clip_id: &str,
    aspect_ratio: &str,
) -> Result<RecoveryResult, AppError> {
    let mut session = match prepare_recovery(db, clip_id, aspect_ratio)? {
        RecoveryPlan::Ready(session) => session,
        RecoveryPlan::Resolved(result) => return Ok(result),
    };
    if session.encrypted_uri.is_none() || session.state == "review_required" {
        return require_review(db, &mut session);
    }
    if session.retry_at > chrono::Utc::now().timestamp() {
        return Ok(result(
            &session,
            "retry_later",
            "YouTube requested a delay. Check recovery again after waiting.",
        ));
    }
    let token = super::youtube::ensure_fresh_access_token(db).await?;
    let mut transport = GoogleTransport::new()?;
    recover_session(db, &mut session, &token, &mut transport).await
}

async fn recover_session(
    db: &DbConn,
    session: &mut Session,
    token: &str,
    transport: &mut dyn Transport,
) -> Result<RecoveryResult, AppError> {
    if session.encrypted_uri.is_none() || session.state == "review_required" {
        return require_review(db, session);
    }
    if session.retry_at > chrono::Utc::now().timestamp() {
        return Ok(result(
            session,
            "retry_later",
            "YouTube requested a delay. Check recovery again after waiting.",
        ));
    }
    require_current(db, session, token)?;
    let uri = session_uri(session)?;
    let response = match transport.send(WireRequest{method:Method::Probe,uri:&uri,token,
        range:Some(format!("bytes */{}",session.size)),body:Vec::new(),total:session.size}).await {
        Ok(response)=>response,
        Err(_)=>return retry_later(db,session,None,"YouTube could not confirm the saved session. No new upload was started; check again later."),
    };
    match response.status {
        200|201=>return finish(db,session,&video_id(&response)?),
        404|410=>return require_review(db,session),
        308=>{
            session.offset=acknowledged_offset(response.range.as_deref(),true,session.offset,session.size,session.size)?;
            session.retry_at=retry_deadline(response.retry_after.as_deref(),chrono::Utc::now().timestamp(),0);
            save_progress(db,session)?;
            if session.retry_at>chrono::Utc::now().timestamp() {
                return Ok(result(session,"retry_later","YouTube asked this session to wait before resuming. Check again later."));
            }
        }
        _=>return retry_later(db,session,response.retry_after.as_deref(),"YouTube has not confirmed this session. Check again later; another upload remains blocked."),
    }
    if session.offset == session.size {
        return retry_later(
            db,
            session,
            None,
            "YouTube has all bytes but has not confirmed a video ID. Check again later.",
        );
    }
    let mut artifact = open_artifact(&session.artifact_path).await?;
    if artifact.size != session.size || artifact.sha256 != session.sha256 {
        return Err(AppError::Api("The original rendered video changed. Recovery will not send different bytes to this session. Restore the original file, then check again.".into()));
    }
    transfer(db, session, token, &mut artifact, transport).await
}

pub fn review_absent(
    db: &DbConn,
    clip_id: &str,
    aspect_ratio: &str,
    review_id: &str,
    target_account_id: &str,
    confirmed_absent: bool,
) -> Result<(), AppError> {
    if !confirmed_absent {
        return Err(AppError::Api(
            "Confirm that you checked YouTube Studio and this video is absent.".into(),
        ));
    }
    let conn = lock_db(db)?;
    let session = latest(&conn, clip_id, aspect_ratio)?
        .ok_or_else(|| AppError::Api("Check upload recovery before confirming absence.".into()))?;
    check_fence(&conn, &session)?;
    db::validate_upload_destination(&conn, "youtube", Some(target_account_id))
        .map_err(AppError::Api)?;
    if session.state != "review_required"
        || session.review_id.as_deref() != Some(review_id)
        || session.account_id != target_account_id
    {
        return Err(AppError::Api(
            "This review is stale or belongs to another account. Check upload recovery again."
                .into(),
        ));
    }
    transaction(&conn, || {
        check_fence(&conn, &session)?;
        let now = chrono::Utc::now().to_rfc3339();
        let changed=conn.execute("UPDATE youtube_upload_sessions SET state='reviewed_absent',review_id=NULL,
            reviewed_absent_at=?3,updated_at=?3 WHERE attempt_id=?1 AND review_id=?2 AND state='review_required'",
            params![session.attempt_id,review_id,now])?;
        if changed != 1 {
            return Err(AppError::Api(
                "This upload review has already been used.".into(),
            ));
        }
        let cleared=conn.execute("UPDATE upload_history SET status='failed',job_id=NULL,
            last_error='User checked YouTube Studio and confirmed this video absent; a fresh upload is allowed.',updated_at=?3
            WHERE id=?1 AND job_id=?2",params![session.history_id,session.attempt_id,now])?;
        if cleared != 1 {
            return Err(AppError::Api(
                "The upload changed during its absence review.".into(),
            ));
        }
        Ok(())
    })
}
