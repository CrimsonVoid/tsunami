mod bencode;
mod torrent;
mod utils;

pub fn do_nothing() -> Option<()> {
    torrent::Torrent::decode("hello").map(|_| ())
}
