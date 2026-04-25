use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Ffmpeg(#[from] rsmpeg::error::RsmpegError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("NulError: {0}")]
    Nul(#[from] std::ffi::NulError),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Extension trait to convert `Result<T, E>` or `Option<T>` into `Result<T, Error>`
/// with a context message, mimicking `anyhow::Context`.
pub trait ResultExt<T>: Sized {
    fn context(self, msg: &str) -> Result<T>;
}

impl<T, E: std::fmt::Display> ResultExt<T> for std::result::Result<T, E> {
    fn context(self, msg: &str) -> Result<T> {
        self.map_err(|e| Error::Other(format!("{msg}: {e}")))
    }
}

impl<T> ResultExt<T> for Option<T> {
    fn context(self, msg: &str) -> Result<T> {
        self.ok_or_else(|| Error::Other(msg.to_string()))
    }
}
