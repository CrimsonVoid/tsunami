use std::{env::temp_dir, path::PathBuf};

use dirs::{download_dir as downld_dir, home_dir};
use hyper::{body, body::Bytes, client::HttpConnector, Client};
use once_cell::sync::Lazy;

use crate::error::Result;

pub type Slice<T> = Box<[T]>;

pub async fn get_body(url: &str) -> Result<Bytes> {
    static CLIENT: Lazy<Client<HttpConnector>> = Lazy::new(|| Client::new());

    let uri = url.parse()?;
    let resp = CLIENT.get(uri).await?;
    Ok(body::to_bytes(resp).await?)
}

pub fn valid_path(p: &str) -> bool {
    // todo: should we check for invalid paths? (incl os-specific blacklists) ?

    p != "." && p != ".." && p != ""
}

pub fn download_dir() -> PathBuf {
    downld_dir().or_else(home_dir).unwrap_or_else(temp_dir)
}
