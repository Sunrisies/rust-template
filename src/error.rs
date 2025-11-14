use actix_web::{HttpResponse, ResponseError};
use thiserror::Error;
use zip::result::ZipError;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Multipart error: {0}")]
    Multipart(#[from] actix_multipart::MultipartError),

    #[error("Event processing error: {0}")]
    EventProcessing(String),

    #[error("Other error: {0}")]
    NotFound(String),
    #[error("ZipError error: {0}")]
    ZipError(String),
}

impl ResponseError for AppError {
    fn error_response(&self) -> HttpResponse {
        match self {
            AppError::Database(_) => {
                HttpResponse::InternalServerError().json(crate::models::ApiResponse::<()> {
                    code: 500,
                    status: 500,
                    message: "Database error".to_string(),
                    data: (),
                })
            }
            AppError::Io(_) => {
                HttpResponse::InternalServerError().json(crate::models::ApiResponse::<()> {
                    code: 500,
                    status: 500,
                    message: "IO error".to_string(),
                    data: (),
                })
            }
            AppError::EventProcessing(msg) => {
                HttpResponse::BadRequest().json(crate::models::ApiResponse::<()> {
                    code: 400,
                    status: 400,
                    message: msg.clone(),
                    data: (),
                })
            }

            _ => HttpResponse::InternalServerError().json(crate::models::ApiResponse::<()> {
                code: 500,
                status: 500,
                message: self.to_string(),
                data: (),
            }),
        }
    }
}
impl From<ZipError> for AppError {
    fn from(err: ZipError) -> Self {
        AppError::ZipError(err.to_string())
    }
}
