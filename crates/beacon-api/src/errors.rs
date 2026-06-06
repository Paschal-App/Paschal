use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use beacon_db as db;
use serde_json::json;

/// User-visible error envelope. Renders as RFC 7807 problem+json.
#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorised,
    NotFound,
    Conflict(String),
    Forbidden(String),
    /// Internal: hidden from the user, logged in detail.
    Internal(anyhow::Error),
}

impl ApiError {
    #[allow(dead_code)]
    pub fn internal<E: Into<anyhow::Error>>(e: E) -> Self {
        Self::Internal(e.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, title, detail) = match self {
            Self::BadRequest(d) => (StatusCode::BAD_REQUEST, "bad_request", d),
            Self::Unauthorised => (StatusCode::UNAUTHORIZED, "unauthorised", String::new()),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found", String::new()),
            Self::Conflict(d) => (StatusCode::CONFLICT, "conflict", d),
            Self::Forbidden(d) => (StatusCode::FORBIDDEN, "forbidden", d),
            Self::Internal(e) => {
                tracing::error!(error = ?e, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "unexpected error".into(),
                )
            }
        };
        let body = json!({
            "type": format!("https://paschal.com/errors/{}", title),
            "title": title,
            "status": status.as_u16(),
            "detail": detail,
        });
        (
            status,
            [(header::CONTENT_TYPE, "application/problem+json")],
            Json(body),
        )
            .into_response()
    }
}

impl From<db::DbError> for ApiError {
    fn from(e: db::DbError) -> Self {
        match e {
            db::DbError::NotFound => Self::NotFound,
            other => Self::Internal(anyhow::anyhow!(other)),
        }
    }
}

impl From<crypto_stub::CryptoError> for ApiError {
    fn from(e: crypto_stub::CryptoError) -> Self {
        Self::Internal(anyhow::anyhow!(e))
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        Self::Internal(e)
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
