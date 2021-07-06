mod bencode;
mod download;
mod error;
mod torrent;
mod utils;

pub async fn do_nothing() -> Option<()> {
    let t = download::Tsunami::new("")?;
    let _peers = t.tracker_handshake().await.ok()?;

    None
}
