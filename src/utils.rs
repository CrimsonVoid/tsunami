use std::{env::temp_dir, path::PathBuf};

use dirs::{download_dir as downld_dir, home_dir};
use hyper::{Client, body, body::Bytes, client::HttpConnector};
use once_cell::sync::Lazy;

use crate::error::Result;

pub type Slice<T> = Box<[T]>;

pub async fn get_body(url: &str) -> Result<Bytes> {
    static CLIENT: Lazy<Client<HttpConnector>> = Lazy::new(Client::new);

    let uri = url.parse()?;
    let resp = CLIENT.get(uri).await?;
    Ok(body::to_bytes(resp).await?)
}

pub fn valid_path(p: &str) -> bool {
    // todo: should we check for invalid paths? (incl os-specific blacklists) ?
    p != "" && p != "." && p != ".."
}

pub fn download_dir() -> PathBuf {
    downld_dir().or_else(home_dir).unwrap_or_else(temp_dir)
}

macro_rules! enum_conv {
    ($generic:ident $(< $( $gen:tt ),+ >)? :: $variant:ident, $type:ty) => {
        impl $(< $($gen),+ >)* From<$type> for $generic $(< $($gen),+ >)* {
            fn from(value: $type) -> Self {
                $generic::$variant(value)
            }
        }

        impl $(< $($gen),+ >)* TryFrom<$generic $(< $($gen),+ >)*> for $type {
            type Error = ();

            fn try_from(value: $generic $(< $($gen),+ >)*) -> Result<Self, Self::Error> {
                match value {
                    $generic::$variant(v) => Ok(v),
                    _ => Err(()),
                }
            }
        }
    };
}

pub(crate) use enum_conv;
