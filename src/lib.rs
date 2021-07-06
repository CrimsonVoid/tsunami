mod bencode;
mod torrent;
mod utils;
mod download;

pub fn do_nothing() -> Option<()> {
    torrent::Torrent::decode("hello").map(|_| ())
}
