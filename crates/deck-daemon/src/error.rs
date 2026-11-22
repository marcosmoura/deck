//! Errors of [`Daemon::run`](crate::Daemon::run).

use std::io;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("deck daemon is already running")]
    AlreadyRunning,
    #[error("another deck daemon uses the state in {}", .0.display())]
    StateInUse(PathBuf),
    #[error("unsafe runtime directory {}: {reason}", path.display())]
    UnsafeRuntimeDir { path: PathBuf, reason: String },
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
}

impl DaemonError {
    pub(crate) fn io(context: impl Into<String>, source: io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}
