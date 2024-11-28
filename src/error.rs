use std::{error as err, fmt, io, result::Result as StdResult};

use crate::utils::enum_conv;

pub type Result<O, E = Error> = StdResult<O, E>;

#[derive(Debug)]
pub enum Error {
    InvalidTrackerResp(Option<String>),
    NoTrackerAvailable,
    Reqwest(reqwest::Error),
}

enum_conv!(Error::Reqwest, reqwest::Error);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidTrackerResp(_) => f.write_str("tracker sent an invalid response"),
            Error::NoTrackerAvailable => f.write_str("exhausted all available trackers"),
            Error::Reqwest(e) => f.write_fmt(format_args!("reqwest error {e}")),
        }
    }
}

impl err::Error for Error {
    fn source(&self) -> Option<&(dyn err::Error + 'static)> {
        match self {
            Error::InvalidTrackerResp(_) | Error::NoTrackerAvailable => None,
            Error::Reqwest(e) => Some(e),
        }
    }
}

#[derive(Debug)]
pub enum DecodeError {
    Io(io::Error),
    MessageId(u8, u32),
}

enum_conv!(DecodeError::Io, io::Error);

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::Io(e) => f.write_fmt(format_args!("io error: {e}")),
            DecodeError::MessageId(id, len) => {
                f.write_fmt(format_args!("unknown message id {id} (len: {len})"))
            }
        }
    }
}

impl err::Error for DecodeError {
    fn source(&self) -> Option<&(dyn err::Error + 'static)> {
        match self {
            DecodeError::Io(e) => Some(e),
            DecodeError::MessageId(_, _) => None,
        }
    }
}
