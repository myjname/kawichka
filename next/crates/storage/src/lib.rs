//! Explicit-source storage operations; never select or switch live storage implicitly.
mod datafs;
pub mod delivery;
pub mod guard;
pub mod library;
pub mod location;
pub mod presets;
pub mod preview;
pub mod profile;
pub mod sandbox;
pub mod support;
pub mod transfer;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("storage I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Settings(#[from] kavychka_core::legacy::SettingsError),
    #[error("storage check failed: {0}")]
    Invalid(String),
    #[error("operation cancelled; original files are unchanged")]
    Cancelled,
    #[error("source changed; use a new preview destination")]
    SourceChanged,
    #[error("storage or legacy application is busy")]
    Busy,
}

pub type Result<T> = std::result::Result<T, Error>;
