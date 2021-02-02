mod bencode;
mod torrent;

pub fn do_nothing() -> Option<()> {
    torrent::Torrent::decode("hello").map(|_| ())
}
