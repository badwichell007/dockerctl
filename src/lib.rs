pub mod audit;
pub mod cli;
pub mod config;
pub mod docker;
pub mod domain;
pub mod health;
pub mod ops;
pub mod output;
pub mod telemetry;
pub mod tui;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Docker(#[from] bollard::errors::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    TomlDe(#[from] toml::de::Error),
    #[error(transparent)]
    TomlSer(#[from] toml::ser::Error),
}

pub fn msg<T>(message: impl Into<String>) -> AppResult<T> {
    Err(AppError::Message(message.into()))
}
