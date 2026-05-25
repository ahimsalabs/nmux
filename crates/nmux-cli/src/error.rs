use std::io;

use nmux_core::host::HostError;
use nmux_proto::wire::WireError;

#[derive(Debug, thiserror::Error)]
pub(crate) enum ServeError {
    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Wire(#[from] WireError),

    #[error(transparent)]
    Host(#[from] HostError),

    #[error("session shutdown")]
    SessionShutdown,

    #[error("{0}")]
    Other(String),

    /// Preserves a boxed error for downstream downcast (e.g. `ServerError`).
    #[error("{0}")]
    Boxed(Box<dyn std::error::Error>),
}

impl From<flatbuffers::InvalidFlatbuffer> for ServeError {
    fn from(err: flatbuffers::InvalidFlatbuffer) -> Self {
        Self::Other(err.to_string())
    }
}

impl From<Box<dyn std::error::Error>> for ServeError {
    fn from(err: Box<dyn std::error::Error>) -> Self {
        Self::Boxed(err)
    }
}

impl ServeError {
    /// Convert to a boxed error, unwrapping the inner box for `Boxed` variant
    /// to preserve downcast capability.
    pub(crate) fn into_boxed(self) -> Box<dyn std::error::Error> {
        match self {
            Self::Boxed(inner) => inner,
            other => Box::new(other),
        }
    }
}

impl From<&str> for ServeError {
    fn from(msg: &str) -> Self {
        Self::Other(msg.to_owned())
    }
}

impl From<String> for ServeError {
    fn from(msg: String) -> Self {
        Self::Other(msg)
    }
}
