use std::{env::temp_dir, path::PathBuf};

use hyper::{body, body::Bytes, client::HttpConnector, Client};
use lazy_static::lazy_static;

use crate::error::Result;

pub(crate) async fn get_body(url: &str) -> Result<Bytes> {
    lazy_static! {
        static ref CLIENT: Client<HttpConnector> = Client::new();
    }

    let uri = url.parse()?;
    let resp = CLIENT.get(uri).await?;
    Ok(body::to_bytes(resp).await?)
}

pub(crate) fn valid_path(p: &str) -> bool {
    // todo: should we check for invalid paths? (incl os-specific blacklists) ?

    p != "." && p != ".." && p != ""
}

pub(crate) fn download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(temp_dir)
}
