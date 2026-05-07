//! HTTP-friendly error type for the workspaces domain.
//!
//! Mirrors the experiments domain so the response shape is uniform
//! across the PhD adapter.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid input: {0}")]
    BadRequest(String),
    #[error("invalid manifest: {0}")]
    InvalidManifest(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("internal error: {0}")]
    Internal(String),
}

impl WorkspaceError {
    fn status(&self) -> StatusCode {
        match self {
            Self::NotFound(_) => StatusCode::NOT_FOUND,
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::InvalidManifest(_) => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::Io(_) | Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::BadRequest(_) => "bad_request",
            Self::InvalidManifest(_) => "invalid_manifest",
            Self::Conflict(_) => "conflict",
            Self::Io(_) => "io_error",
            Self::Internal(_) => "internal",
        }
    }
}

impl IntoResponse for WorkspaceError {
    fn into_response(self) -> Response {
        let body = Json(json!({
            "error": { "code": self.code(), "message": self.to_string() }
        }));
        (self.status(), body).into_response()
    }
}

impl From<serde_json::Error> for WorkspaceError {
    fn from(v: serde_json::Error) -> Self {
        Self::InvalidManifest(v.to_string())
    }
}

pub type WorkspaceResult<T> = Result<T, WorkspaceError>;
