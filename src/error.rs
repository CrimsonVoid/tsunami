use std::{io, result::Result as StdResult};

use hyper::http::uri::InvalidUri;
use thiserror::Error;

pub type Result<O, E = Error> = StdResult<O, E>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("tracker sent an invalid response")]
    InvalidTrackerResp { reason: Option<String> },

    #[error("exhausted all available trackers")]
    NoTrackerAvailable,

    #[error("invalid tracker uri")]
    InvalidTrackerUri(#[from] InvalidUri),

    #[error("hyper error")]
    Hyper(#[from] hyper::Error),
}

#[derive(Debug, Error)]
crate enum DecodeError {
    #[error("io error")]
    Io(#[from] io::Error),

    #[error("unknown message id {0} (len: {1})")]
    MessageId(u8, u32),
}
