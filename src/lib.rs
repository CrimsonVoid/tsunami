mod bencode;

pub fn do_nothing() -> Option<()> {
    bencode::Bencode::decode("hello").ok().map(|_| ())
}
