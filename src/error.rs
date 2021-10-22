use hyper::http::uri::InvalidUri;
use std::result::Result as StdResult;
use thiserror::Error;

pub type Result<O> = StdResult<O, Error>;

#[derive(Error, Debug)]
pub enum Error {
    #[error("tracker sent an invalid response")]
    InvalidTrackerResp { failure_reason: Option<String> },

    #[error("exhausted all available trackers")]
    NoTrackerAvailable,

    #[error("invalid tracker uri")]
    InvalidTrackerUri(InvalidUri),

    #[error("hyper error")]
    HyperError(hyper::Error),
}

impl From<InvalidUri> for Error {
    fn from(e: InvalidUri) -> Self {
        Error::InvalidTrackerUri(e)
    }
}

impl From<hyper::Error> for Error {
    fn from(e: hyper::Error) -> Self {
        Error::HyperError(e)
    }
}
