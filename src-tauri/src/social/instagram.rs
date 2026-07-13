// TODO(v2): Instagram adapter — pending Meta developer app review

use crate::error::AppError;
use crate::social::{ConnectedAccount, PlatformAdapter, UploadMeta, UploadResult};
use rusqlite::Connection;

pub struct InstagramAdapter;

#[async_trait::async_trait(?Send)]
impl PlatformAdapter for InstagramAdapter {
    fn platform_id(&self) -> &'static str {
        "instagram"
    }

    fn is_ready(&self, _db: &Connection) -> Result<bool, AppError> {
        Ok(false)
    }

    async fn start_auth(&self) -> Result<String, AppError> {
        Err(AppError::NotSupported(
            "Instagram publishing coming soon".into(),
        ))
    }

    async fn handle_callback(
        &self,
        _db: &crate::DbConn,
        _code: &str,
    ) -> Result<ConnectedAccount, AppError> {
        Err(AppError::NotSupported(
            "Instagram publishing coming soon".into(),
        ))
    }

    async fn refresh_token(&self, _db: &crate::DbConn) -> Result<(), AppError> {
        Err(AppError::NotSupported(
            "Instagram publishing coming soon".into(),
        ))
    }

    async fn upload_video(
        &self,
        _db: &crate::DbConn,
        _file_path: &str,
        _meta: &UploadMeta,
    ) -> Result<UploadResult, AppError> {
        Err(AppError::NotSupported(
            "Instagram publishing coming soon".into(),
        ))
    }

    fn disconnect(&self, _db: &Connection) -> Result<(), AppError> {
        Ok(())
    }

    fn get_account(&self, _db: &Connection) -> Result<Option<ConnectedAccount>, AppError> {
        Ok(None)
    }
}
