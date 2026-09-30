use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("HTTP {status} for {url}")]
    Status { status: u16, url: String },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),
    #[error("playlist parse error: {0}")]
    Parse(String),
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("decryption failed: {0}")]
    Decrypt(String),
    #[error("no variant matches quality '{0}'")]
    NoVariant(String),
    #[error("stream is not live yet: {0}")]
    NotLive(String),
    /// The output or work directory already holds something this run must not overwrite.
    #[error("{0}")]
    Conflict(String),
    #[error("ffmpeg failed: {0}")]
    Mux(String),
    #[error("cancelled; completed segments were kept, rerun the same command to resume")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;
