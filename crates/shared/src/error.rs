use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("ai provider error ({status}): {body}")]
    AiProvider { status: u16, body: String },
    #[error("ai provider unavailable: {0}")]
    AiUnavailable(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("pipeline error: {0}")]
    Pipeline(String),
    #[error("pdf parse error: {0}")]
    Pdf(String),
    #[error("csv error: {0}")]
    Csv(#[from] csv::Error),
    #[error("config error: {0}")]
    Config(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn status_code(&self) -> u16 {
        match self {
            Error::NotFound(_) => 404,
            Error::BadRequest(_) | Error::Csv(_) | Error::Config(_) => 400,
            Error::AiUnavailable(_) => 503,
            Error::AiProvider { status, .. } if *status >= 400 && *status < 500 => *status,
            _ => 500,
        }
    }
}

// sqlx/reqwest are referenced through workspace deps re-exported here so
// downstream crates only depend on `shared` for error handling.
pub use reqwest;
pub use sqlx;
