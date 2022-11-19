use std::{error as err, fmt, io, result::Result as StdResult};

use hyper::http::uri::InvalidUri;

use crate::utils::enum_conv;

pub type Result<O, E = Error> = StdResult<O, E>;

#[derive(Debug)]
pub enum Error {
    InvalidTrackerResp(Option<String>),
    NoTrackerAvailable,
    InvalidTrackerUri(InvalidUri),
    Hyper(hyper::Error),
}

enum_conv!(Error::InvalidTrackerUri, InvalidUri);
enum_conv!(Error::Hyper, hyper::Error);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidTrackerResp(_) => f.write_str("tracker sent an invalid response"),
            Error::NoTrackerAvailable => f.write_str("exhausted all available trackers"),
            Error::InvalidTrackerUri(e) => f.write_fmt(format_args!("invalid tracker uri {e}")),
            Error::Hyper(e) => f.write_fmt(format_args!("hyper error {e}")),
        }
    }
}

impl err::Error for Error {
    fn source(&self) -> Option<&(dyn err::Error + 'static)> {
        match self {
            Error::InvalidTrackerResp(_) | Error::NoTrackerAvailable => None,
            Error::InvalidTrackerUri(e) => Some(e),
            Error::Hyper(e) => Some(e),
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
