use hyper::http::uri::InvalidUri;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TError {
    #[error("tracker sent an invalid response")]
    InvalidTrackerResp { failure_reason: Option<String> },

    #[error("tracker uri is invalid")]
    InvalidUri(#[from] InvalidUri),

    #[error("hyper error")]
    Hyper(#[from] hyper::Error),
}

pub type TResult<O> = Result<O, TError>;
