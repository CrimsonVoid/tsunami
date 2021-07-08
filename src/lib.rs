mod bencode;
mod download;
mod error;
mod torrent;
mod utils;

pub async fn do_nothing() -> Option<()> {
    let mut t = download::Tsunami::new(b"")?;
    let _peers = t.tracker_handshake().await.ok()?;

    None
}
