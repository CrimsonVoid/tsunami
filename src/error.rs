use thiserror::Error;

#[derive(Error, Debug)]
pub enum TError {
    #[error("tracker sent an invalid response")]
    InvalidTrackerResp { failure_reason: Option<String> },

    #[error("exhausted all available trackers")]
    NoTrackerAvailable,
}

pub type TResult<O> = Result<O, TError>;
