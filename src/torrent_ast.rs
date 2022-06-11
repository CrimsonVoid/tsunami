use std::collections::HashMap;

use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::{char as nchar, digit0, digit1, one_of},
    combinator::{map, map_opt, opt, recognize},
    multi::{length_data, many0},
    sequence::{delimited, terminated, tuple},
};
use ring::digest;

// TorrentAST is a structural representation of a torrent file; fields map over almost identically,
// with dict's being represented as sub-structs
#[derive(Debug, PartialEq)]
pub(crate) struct TorrentAST<'a> {
    pub(crate) announce: &'a str,
    pub(crate) announce_list: Option<Vec<Vec<&'a str>>>,
    pub(crate) info: InfoAST<'a>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct InfoAST<'a> {
    pub(crate) piece_length: i64,
    pub(crate) pieces: &'a [u8],
    pub(crate) private: Option<i64>,
    pub(crate) name: &'a str,

    // length and files are mutually exclusive
    // single file case
    pub(crate) length: Option<i64>,
    // multi-file case
    pub(crate) files: Option<Vec<FileAST<'a>>>,
}

#[derive(Debug, PartialEq)]
pub(crate) struct FileAST<'a> {
    pub(crate) path: Vec<&'a str>,
    pub(crate) length: i64,
}

impl<'a> TorrentAST<'a> {
    pub(crate) fn decode(file: &'a [u8]) -> Option<TorrentAST<'a>> {
        let mut torrent = Bencode::decode(file)?.dict()?;
        let mut info = torrent.remove(&b"info"[..])?.dict()?;

        TorrentAST {
            announce: torrent.remove(&b"announce"[..])?.str()?,
            announce_list: try {
                torrent
                    .remove(&b"announce-list"[..])?
                    .map_list(|l| l.map_list(Bencode::str))?
            },
            info: InfoAST {
                name: info.remove(&b"name"[..])?.str()?,
                pieces: info.remove(&b"pieces"[..])?.bstr()?,
                piece_length: info.remove(&b"piece length"[..])?.num()?,

                length: try { info.remove(&b"length"[..])?.num()? },
                files: try { info.remove(&b"files"[..])?.map_list(FileAST::new)? },
                private: try { info.remove(&b"private"[..])?.num()? },
            },
        }
        .validate()
    }

    fn validate(self) -> Option<TorrentAST<'a>> {
        // pieces is a list of 20 byte sha1 hashes
        if self.info.pieces.len() % 20 != 0 {
            return None;
        }

        // we can have at most 2^32 pieces. this limit is not directly defined but since index
        // in a Peer's Request message is limited to u32 we can infer there must be fewer than
        // 2^32 pieces.
        if self.info.pieces.len() > u32::MAX as usize {
            return None;
        }

        // length and files are mutually exclusive for a valid torrent
        if self.info.length.is_some() && self.info.files.is_some() {
            return None;
        } else if self.info.length.is_none() && self.info.files.is_none() {
            return None;
        }

        Some(self)
    }
}

impl<'a> FileAST<'a> {
    fn new(benc: Bencode) -> Option<FileAST> {
        let mut file = benc.dict()?;

        Some(FileAST {
            path: file.remove(&b"path"[..])?.map_list(|p| p.str())?,
            length: file.remove(&b"length"[..])?.num()?,
        })
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Bencode<'a> {
    Num(i64),
    Str(&'a str),
    BStr(&'a [u8]),
    List(Vec<Bencode<'a>>),
    Dict(HashMap<&'a [u8], Bencode<'a>>),
}

impl<'a> Bencode<'a> {
    /// decode a bencoded value consuming all of input in the process
    ///
    /// # Failures
    /// This function fails if any of the input is left after producing a value
    ///
    /// # Examples
    /// ```ignore
    /// # use tsunami::torrent_ast::Bencode;
    /// assert!(Bencode::decode(b"i42e").unwrap() == Bencode::Num(42));
    ///
    /// // consumed an empty dict but there was input left
    /// assert!(Bencode::decode(b"i42e ") == None);
    /// ```
    pub(crate) fn decode(input: &[u8]) -> Option<Bencode> {
        // make sure we consumed the whole input
        let Ok((&[], benc)) = Bencode::parse_benc(input) else {
            return None
        };

        Some(benc)
    }

    /// compute the SHA-1 hash of a dictionary in input
    ///
    /// # Examples
    /// ```ignore
    /// # use tsunami::torrent_ast::Bencode;
    ///
    /// let input = b"d4:infod5:helloi2eee";
    /// let expected = Some([
    ///       3, 245, 131,  59,  43,
    ///     101,  84,   9, 152, 153,
    ///     139,  69, 214, 205,  74,
    ///     149, 138, 168,  35,  80, ]);
    ///
    /// assert!(Bencode::hash_dict(&input[..], "info") == expected);
    /// ```
    pub(crate) fn hash_dict(input: &[u8], key: &str) -> Option<[u8; 20]> {
        // SHA-1 hash includes surrounding 'd' and 'e' tags
        //
        // let input         = "d ... 4:infod ... e ... e";
        // let (start, end)  =     start -> [     ] <- end
        //
        // sha1.sum( input[start..=end] )

        map(
            delimited(
                tag("d"),
                many0(tuple((Bencode::parse_str, Bencode::parse_benc_no_map))),
                tag("e"),
            ),
            |kv_pairs| {
                kv_pairs
                    .iter()
                    .find(|(k, _)| *k == key.as_bytes())
                    .map(|(_, v)| {
                        digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, v)
                            .as_ref()
                            .try_into()
                            .unwrap()
                    })
            },
        )(input)
        .ok()?
        .1
    }

    /// str unwraps a [Bencode::Str] variant
    ///
    /// # Examples
    /// ```ignore
    /// # use tsunami::torrent_ast::Bencode;
    ///
    /// assert!(Bencode::Str("str").str() == Some("str"));
    /// assert!(Bencode::BStr(b"str").str() == None);
    /// ```
    pub(crate) fn str(self) -> Option<&'a str> {
        match self {
            Bencode::Str(s) => Some(s),
            _ => None,
        }
    }

    /// bstr unwraps a [Bencode::BStr] variant
    ///
    /// # Examples
    /// ```ignore
    /// # use tsunami::torrent_ast::Bencode;
    ///
    /// assert!(Bencode::BStr(b"str").bstr() == Some(&b"str"[..]));
    /// assert!(Bencode::Str("str").bstr() == None);
    /// ```
    pub(crate) fn bstr(self) -> Option<&'a [u8]> {
        match self {
            Bencode::BStr(s) => Some(s),
            _ => None,
        }
    }

    /// num unwraps a [Bencode::Num] variant
    ///
    /// # Examples
    /// ```ignore
    /// # use tsunami::torrent_ast::Bencode;
    ///
    /// assert!(Bencode::Num(32).num() == Some(32));
    /// # assert!(Bencode::Str("str").num() == None);
    /// ```
    pub(crate) fn num(self) -> Option<i64> {
        match self {
            Bencode::Num(n) => Some(n),
            _ => None,
        }
    }

    /// list unwraps a [Bencode::List] variant
    ///
    /// # Examples
    /// ```ignore
    /// # use tsunami::torrent_ast::Bencode;
    ///
    /// let nums = || vec![Bencode::Num(1 << 42)];
    /// let benc = Bencode::List(nums());
    ///
    /// assert!(benc.list() == Some(nums()));
    /// # assert!(Bencode::Str("str").list() == None);
    /// ```
    pub(crate) fn list(self) -> Option<Vec<Bencode<'a>>> {
        match self {
            Bencode::List(v) => Some(v),
            _ => None,
        }
    }

    /// dict unwraps a [Bencode::Dict] variant
    ///
    /// # Examples
    /// ```ignore
    /// # use std::collections::HashMap;
    /// # use tsunami::torrent_ast::Bencode;
    ///
    /// let dict = || { HashMap::from([ ("num", Bencode::Num(32)) ]) };
    /// let benc = Bencode::Dict(dict());
    ///
    /// assert!(benc.dict() == Some(dict()));
    /// # assert!(Bencode::Str("str").dict() == None);
    /// ```
    pub(crate) fn dict(self) -> Option<HashMap<&'a [u8], Bencode<'a>>> {
        match self {
            Bencode::Dict(d) => Some(d),
            _ => None,
        }
    }

    /// map_list calls op with every element of a [Bencode::List], returning None if any call to
    /// op returned None
    ///
    /// # Examples
    /// ```ignore
    /// # use tsunami::torrent_ast::Bencode as B;
    ///
    /// let list = || vec![ B::Num(0), B::Num(1), B::Str("two") ];
    /// let benc = || B::List(list());
    ///
    /// assert!(benc().map_list(|b| Some(b)) == Some(list()));
    /// assert!(benc().map_list(|b| b.num()) == None);
    /// ```
    pub(crate) fn map_list<U>(self, op: impl Fn(Bencode<'a>) -> Option<U>) -> Option<Vec<U>> {
        self.list()?.into_iter().map(op).try_collect()
    }
}

type Parsed<'a, T> = nom::IResult<&'a [u8], T>;

impl<'a> Bencode<'a> {
    // nom bencode parsers

    fn parse_benc(input: &'a [u8]) -> Parsed<Bencode> {
        alt((
            map(Self::parse_str, Bencode::wrap_str),
            map(Self::parse_int, Bencode::Num),
            map(Self::parse_list, Bencode::List),
            map(Self::parse_dict, Bencode::Dict),
        ))(input)
    }

    /// attempts to wrap s as either [Bencode::Str] if s is a valid utf8 string or [Bencode::BStr]
    fn wrap_str(s: &[u8]) -> Bencode {
        match std::str::from_utf8(s) {
            Ok(s) => Bencode::Str(s),
            Err(_) => Bencode::BStr(s),
        }
    }

    /// parse a valid bencoded string
    ///
    /// a bencoded string is a base-ten length followed by a colon (:) and then the string
    ///
    /// # Examples
    /// ``` ignore
    /// # use tsunami::torrent_ast::Bencode;
    /// assert!(Bencode::parse_str(b"5:hello").unwrap().1 == &b"hello"[..]);
    /// ```
    fn parse_str(input: &[u8]) -> Parsed<&[u8]> {
        length_data(terminated(
            map_opt(digit1, |n: &[u8]| {
                std::str::from_utf8(n).ok()?.parse::<usize>().ok()
            }),
            nchar(':'),
        ))(input)
    }

    /// parse a valid bencoded int
    ///
    /// pseudo format: i(\d+)e
    /// invalid numbers:
    ///   - i-0e
    ///   - all encodings with a leading zero, eg. i02e
    ///
    /// parsing rules:
    ///   - if a number starts with zero, no digits can follow it. the next tag must be "e"
    ///   - all valid, non-zero numbers must start with a non-zero digit and be
    ///     followed by zero or more digits. regex: (-?[1-9][0-9]+)
    fn parse_int(input: &[u8]) -> Parsed<i64> {
        map_opt(
            delimited(
                nchar('i'),
                alt((
                    tag("0"),
                    recognize(tuple((opt(nchar('-')), one_of("123456789"), digit0))),
                )),
                nchar('e'),
            ),
            |num: &[u8]| std::str::from_utf8(num).ok()?.parse().ok(),
        )(input)
    }

    // parse a valid bencoded list
    // pseudo format: l(Benc)*e
    fn parse_list(input: &'a [u8]) -> Parsed<Vec<Bencode>> {
        delimited(nchar('l'), many0(Self::parse_benc), nchar('e'))(input)
    }

    // parse a valid bencoded dict
    // dict keys must appear in sorted order
    //
    // pseudo format: d(Str Benc)*e
    fn parse_dict(input: &'a [u8]) -> Parsed<HashMap<&[u8], Bencode>> {
        map_opt(
            delimited(
                nchar('d'),
                many0(tuple((Self::parse_str, Self::parse_benc))),
                nchar('e'),
            ),
            |kv_pairs: Vec<(&[u8], Bencode)>| {
                kv_pairs
                    .windows(2)
                    .all(|p| p[0].0 < p[1].0)
                    .then(|| kv_pairs.into_iter().collect())
            },
        )(input)
    }

    // same as parse benc, but doesn't try to parse the resulting str's into Benc nodes
    // unfortunately we have to re-define all of the rules here :(
    fn parse_benc_no_map(input: &'a [u8]) -> Parsed<&[u8]> {
        alt((
            Self::parse_str,
            // int
            recognize(delimited(
                nchar('i'),
                alt((
                    tag("0"),
                    recognize(tuple((opt(nchar('-')), one_of("123456789"), digit0))),
                )),
                nchar('e'),
            )),
            // list
            recognize(delimited(
                nchar('l'),
                many0(Self::parse_benc_no_map),
                nchar('e'),
            )),
            // dict
            recognize(delimited(
                nchar('d'),
                many0(tuple((Self::parse_str, Self::parse_benc_no_map))),
                nchar('e'),
            )),
        ))(input)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::Bencode as B;
    use crate::torrent_ast::Bencode;

    macro_rules! hashmap {
        ($($k:expr => $v:expr),*) => ({
            ::std::collections::HashMap::from([$(($k, $v),)+])
        });

        ($($k:expr => $v:expr),+,) => (hashmap!($($k => $v),+));
    }

    #[test]
    fn parse_int() {
        let cases = vec![
            ("i42e", 42),
            ("i9e", 9),
            ("i0e", 0),
            ("i-5e", -5),
            ("i562949953421312e", 562949953421312),
            ("i-562949953421312e", -562949953421312),
            ("i9223372036854775807e", i64::MAX),
            ("i-9223372036854775808e", i64::MIN),
        ];

        for (input, expected) in cases {
            let actual = B::parse_int(input.as_bytes()).unwrap().1;
            assert_eq!(actual, expected)
        }
    }

    #[test]
    fn parse_int_fail() {
        let cases = vec![
            "e",
            "i-0e",
            "i00e",
            "i05e",
            "i18446744073709551615e",
            "i-0e",
            "i03e",
        ];

        for input in cases {
            assert!(B::parse_int(input.as_bytes()).is_err());
        }
    }

    #[test]
    fn parse_str() {
        let cases = vec![
            ("5:hello", "hello"),
            ("0:", ""),
            ("7:yahallo", "yahallo"),
            ("15:こんにちわ", "こんにちわ"),
            ("7:\"hello\"", "\"hello\""),
            ("11:hellohello1", "hellohello1"),
            ("02:hi", "hi"),
        ];

        for (input, expected) in cases {
            let actual = B::parse_str(input.as_bytes()).unwrap().1;
            assert_eq!(actual, expected.as_bytes())
        }
    }

    #[test]
    fn parse_str_fail() {
        let cases = vec![
            // comment to prevent rustfmt from collapsing cases into a single line :/
            "6:hello",
            "a5:hallo",
            "a",
            "18446744073709551616:overflow",
        ];

        for input in cases {
            assert!(B::parse_str(input.as_bytes()).is_err());
        }
    }

    #[test]
    fn parse_list() {
        let cases = vec![
            ("le", vec![]),
            ("li4ei2e2:42e", vec![B::Num(4), B::Num(2), B::Str("42")]),
            (
                "l5:helloi42eli2ei3e2:hid4:listli1ei2ei3ee7:yahallo2::)eed2:hi5:hello3:inti15eee",
                vec![
                    B::Str("hello"),
                    B::Num(42),
                    B::List(vec![
                        B::Num(2),
                        B::Num(3),
                        B::Str("hi"),
                        B::Dict(hashmap! {
                            &b"list"[..]    => B::List(vec![B::Num(1), B::Num(2), B::Num(3)]),
                            &b"yahallo"[..] => B::Str(":)"),
                        }),
                    ]),
                    B::Dict(hashmap! {
                        &b"hi"[..]  => B::Str("hello"),
                        &b"int"[..] => B::Num(15),
                    }),
                ],
            ),
        ];

        for (input, expected) in cases {
            let actual = B::parse_list(input.as_bytes()).unwrap().1;
            assert_eq!(actual, expected)
        }
    }

    #[test]
    fn parse_dict() {
        let cases = vec![
            ("de", HashMap::new()),
            (
                "d3:onei1e3:twoi2ee",
                hashmap!{ &b"one"[..] => B::Num(1), &b"two"[..] => B::Num(2) },
            ),
            (
                concat!(
                    "d8:announce40:http://tracker.example.com:8080/announce7:comment17:\"Hello mock data",
                    "\"13:creation datei1234567890e9:httpseedsl31:http://direct.example.com/mock131:http",
                    "://direct.example.com/mock2e4:infod6:lengthi562949953421312e4:name15:あいえおう12:p",
                    "iece lengthi536870912eee"),
                hashmap! {
                    &b"announce"[..]      => B::Str("http://tracker.example.com:8080/announce"),
                    &b"comment"[..]       => B::Str("\"Hello mock data\""),
                    &b"creation date"[..] => B::Num(1234567890),
                    &b"httpseeds"[..]     => B::List(vec![
                        B::Str("http://direct.example.com/mock1"),
                        B::Str("http://direct.example.com/mock2"),
                    ]),
                    &b"info"[..] => B::Dict(hashmap!{
                        &b"length"[..]       => B::Num(562949953421312),
                        &b"name"[..]         => B::Str("あいえおう"),
                        &b"piece length"[..] => B::Num(536870912),
                    }),
                }
            ),
        ];

        for (input, expected) in cases {
            let actual = B::parse_dict(input.as_bytes()).unwrap().1;
            assert_eq!(actual, expected)
        }
    }

    #[test]
    fn parse_dict_fail() {
        let cases = vec!["d2:hi5:hello1:ai32ee"];

        for input in cases {
            assert!(B::parse_dict(input.as_bytes()).is_err());
        }
    }

    #[test]
    fn info_hash() {
        let cases = vec![
            (
                concat!(
                    "d8:announce40:http://tracker.example.com:8080/announce7:comment17:\"Hello mock data",
                    "\"13:creation datei1234567890e",
                    // torrent copy
                    "4:demod",
                    "8:announce40:http://tracker.example.com:8080/announce7:comment17:\"Hello mock data",
                    "\"13:creation datei1234567890e",
                    "9:httpseedsl31:http://direct.example.com/mock131:http",
                    "://direct.example.com/mock2e4:infod6:lengthi562949953421312e4:name15:あいえおう12:p",
                    "iece lengthi536870912ee",
                    "e",
                    // torrent copy
                    "9:httpseedsl31:http://direct.example.com/mock131:http",
                    "://direct.example.com/mock2e4:infod6:lengthi562949953421312e4:name15:あいえおう12:p",
                    "iece lengthi536870912eee"
                ).as_bytes(),
                [
                    0x83, 0x55, 0x11, 0x80, 0x8c, 0xd6, 0x54, 0x2c, 0x1b, 0xc5,
                    0x19, 0x8d, 0x2a, 0x48, 0x9d, 0xce, 0xd5, 0x2b, 0x53, 0x3a,
                ],
            ),
            (
                include_bytes!("test_data/mock_dir.torrent"),
                [
                    0x74, 0x53, 0x68, 0x65, 0xe7, 0x7a, 0xcc, 0x72, 0xf2, 0x98,
                    0xc4, 0x88, 0xc3, 0x2c, 0x31, 0xab, 0x9b, 0x96, 0x98, 0xb1,
                ],
            ),
            (
                include_bytes!("test_data/mock_file.torrent"),
                [
                    0x0b, 0x05, 0xab, 0xa1, 0xf2, 0xa0, 0xb2, 0xe6, 0xdc, 0x92,
                    0xf1, 0xdb, 0x11, 0x43, 0x3e, 0x5f, 0x3a, 0x82, 0x0b, 0xad,
                ],
            ),
        ];

        for (torrent, expected) in cases {
            let info_hash = B::hash_dict(torrent, "info").unwrap();
            assert_eq!(info_hash, expected);
        }
    }

    #[test]
    fn decode_bt_test() {
        let test_files = [
            &include_bytes!("test_data/bittorrent-v2-test.torrent")[..],
            &include_bytes!("test_data/bittorrent-v2-hybrid-test.torrent")[..],
        ];

        for file in test_files {
            let torrent = B::decode(file).unwrap();
            print_benc(torrent, 2);
        }
    }

    fn print_benc(v: Bencode, spaces: usize) {
        match v {
            Bencode::Num(_) | Bencode::Str(_) => {
                print!("{v:?},")
            }
            Bencode::BStr(b) => {
                if b.len() >= 20 {
                    let b = &b[..=20];
                    print!("BStr({b:?} ..),");
                } else {
                    print!("BStr({b:?}),");
                }
            }
            Bencode::List(l) => {
                if l.len() < 4 {
                    print!("{l:?},");
                    return;
                }

                println!("List([");
                for node in l {
                    (0..spaces).for_each(|_| print!(" "));
                    print_benc(node, spaces + 2);
                    println!(",");
                }
                (0..spaces - 2).for_each(|_| print!(" "));
                print!("])");
            }
            Bencode::Dict(d) => {
                println!("{{");

                for (k, v) in d {
                    let k = String::from_utf8_lossy(k);
                    (0..spaces).for_each(|_| print!(" "));
                    print!("{k:?} => ");
                    print_benc(v, spaces + 2);
                    println!();
                }

                (0..spaces - 2).for_each(|_| print!(" "));
                print!("}}");
            }
        }
    }
}
