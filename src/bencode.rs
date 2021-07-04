use crate::utils::IterExt;
use nom::character::complete::{digit0, digit1};
use nom::{
    alt, char as nchar, delimited, length_data, many0, map, map_opt, map_res, named, one_of, opt,
    recognize, tag, terminated, tuple,
};
use std::collections::HashMap;

#[derive(Debug, PartialEq)]
pub enum Bencode<'a> {
    Num(i64),
    Str(&'a str),
    List(Vec<Bencode<'a>>),
    Dict(HashMap<&'a str, Bencode<'a>>),
}

impl<'a> Bencode<'a> {
    pub fn decode(input: &str) -> Option<Bencode> {
        match Bencode::nom_benc(input) {
            Ok(("", benc)) => Some(benc), // make sure we consumed the whole input
            _ => None,
        }
    }

    pub fn str(self) -> Option<&'a str> {
        match self {
            Bencode::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn num(self) -> Option<i64> {
        match self {
            Bencode::Num(n) => Some(n),
            _ => None,
        }
    }

    pub fn list(self) -> Option<Vec<Bencode<'a>>> {
        match self {
            Bencode::List(v) => Some(v),
            _ => None,
        }
    }

    pub fn dict(self) -> Option<HashMap<&'a str, Bencode<'a>>> {
        match self {
            Bencode::Dict(d) => Some(d),
            _ => None,
        }
    }

    pub fn map_list<U>(self, op: impl Fn(Bencode<'a>) -> Option<U>) -> Option<Vec<U>> {
        self.list()?.into_iter().flat_map_all(op)
    }

    /*
    // compute a SHA-1 hash of an info dictionary found in torrent_file
    pub fn info_hash(torrent_file: &'a str) -> Option<[u8; 20]> {
        // torrent_file format:
        // d ... 4:infod ... e ... e
        //    start -> [     ] <- end
        //
        // sha1.sum( &torrent_file[start..=end] )

        let mut tokens = <Token as Logos>::lexer(torrent_file).spanned();

        // torrent files are dictionaries
        if !matches!(tokens.next()?, (Token::D, _)) {
            return None;
        }

        let start = {
            // nest_level tracks how deep we are in a record. every time we encounter the start of
            // a bencode token (I, L, or D) we increment nest_level by one. likewise, when we
            // find the end of a token (E) nest_level is decremented by one. this way we can track
            // if we found a top-level "info" key.
            let mut nest_level = 0;
            let mut found = false;

            while !found {
                match tokens.next()?.0 {
                    Token::Str("info") if nest_level == 0 => found = true,
                    Token::I | Token::L | Token::D => nest_level += 1,
                    Token::E => nest_level -= 1,
                    Token::Error => return None,
                    _ => {}
                }
            }

            // found an "info" key, value should be a bencoded dict
            match tokens.next()? {
                (Token::D, range) => range.start,
                _ => return None,
            }
        };

        let end = {
            // we are currently inside the info dict, and need to consume tokens until we get find
            // the closing E token
            let mut nest_level = 1;

            while nest_level > 0 {
                match tokens.next()?.0 {
                    Token::I | Token::L | Token::D => nest_level += 1,
                    Token::E => nest_level -= 1,
                    Token::Error => return None,
                    _ => {}
                }
            }

            // we consumed the closing E tag and can't be sure what the next token will be, but
            // there MUST be at least one more token since we are inside a dictionary
            match tokens.next()? {
                (_, range) => range.start,
            }
        };

        digest::digest(
            &digest::SHA1_FOR_LEGACY_USE_ONLY,
            &torrent_file[start..end].as_bytes(),
        )
        .as_ref()
        .try_into()
        .ok()
    }
    */
}

impl<'a> Bencode<'a> {
    // nom bencode parsers

    named!(
        nom_benc(&'a str) -> Bencode,
        alt!(
            map!(Self::nom_str, Bencode::Str)
                | map!(Self::nom_int, Bencode::Num)
                | map!(Self::nom_list, Bencode::List)
                | map!(Self::nom_dict, Bencode::Dict)
        )
    );

    named!(
        // parse a valid bencoded string
        // bencodded strings are a number followed by a colon (':') and then a string as long as
        // the preceding number
        //
        // pseudo format: \d+:(.*)
        nom_str(&'a str) -> &str,
        length_data!(terminated!(
                map_res!(digit1, |n: &str| n.parse::<usize>()),
                nchar!(':')
        ))
    );

    named!(
        // parse a valid bencoded int
        // pseudo format: i(\d+)e
        // invalid numbers:
        //   - i-0e
        //   - all encodings with a leading zero, eg. i-02e
        //
        // parsing rules:
        //   - if a number starts with zero, no digits can follow it. the next tag must be "e"
        //   - all valid, non-zero numbers must start with a non-zero digit and be
        //     followed by zero or more digits. regex: (-?[1-9][0-9]+)
        nom_int(&'a str) -> i64,
        map_res!(
            delimited!(
                nchar!('i'),
                alt!(
                    tag!("0") | recognize!(tuple!(opt!(nchar!('-')), one_of!("123456789"), digit0))
                ),
                nchar!('e')
            ),
            |num: &str| num.parse()
        )
    );

    named!(
        // parse a valid bencoded list
        // pseudo format: l(Benc)*e
        nom_list(&'a str) -> Vec<Bencode>,
        delimited!(nchar!('l'), many0!(Self::nom_benc), nchar!('e'))
    );

    named!(
        // parse a valid bencoded dict
        // dict keys must appear in sorted order
        //
        // pseudo format: d(Str Benc)*e
        nom_dict(&'a str) -> HashMap<&str, Bencode>,
        map_opt!(
            delimited!(
                nchar!('d'),
                many0!(tuple!(Self::nom_str, Self::nom_benc)),
                nchar!('e')
            ),
            |kv_pairs: Vec<(&'a str, Bencode<'a>)>| {
                kv_pairs
                    .windows(2)
                    .all(|p| p[0].0 < p[1].0)
                    .then(|| kv_pairs.into_iter().collect())
            }
        )
    );
}

#[cfg(test)]
mod tests {
    use super::Bencode as B;
    use std::collections::HashMap;
    use std::str::from_utf8_unchecked;

    macro_rules! hashmap {
        ($($k:expr => $v:expr),*) => ({
            let mut d = ::std::collections::HashMap::new();
            $(d.insert($k, $v);)*
            d
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
            let actual = B::nom_int(input).unwrap().1;
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
            assert!(B::nom_int(input).is_err());
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
            let actual = B::nom_str(input).unwrap().1;
            assert_eq!(actual, expected)
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
            assert!(B::nom_str(input).is_err());
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
                            "list"    => B::List(vec![B::Num(1), B::Num(2), B::Num(3)]),
                            "yahallo" => B::Str(":)"),
                        }),
                    ]),
                    B::Dict(hashmap! {
                        "hi"  => B::Str("hello"),
                        "int" => B::Num(15),
                    }),
                ],
            ),
        ];

        for (input, expected) in cases {
            let actual = B::nom_list(input).unwrap().1;
            assert_eq!(actual, expected)
        }
    }

    #[test]
    fn parse_dict() {
        let cases = vec![
            // comment to prevent rustfmt from collapsing cases into a single line :/
            ("de", HashMap::new()),
            (
                "d3:onei1e3:twoi2ee",
                hashmap! {
                    "one" => B::Num(1),
                    "two" => B::Num(2),
                },
            ),
            (
                concat!(
                    "d8:announce40:http://tracker.example.com:8080/announce7:comment17:\"Hello mock data",
                    "\"13:creation datei1234567890e9:httpseedsl31:http://direct.example.com/mock131:http",
                    "://direct.example.com/mock2e4:infod6:lengthi562949953421312e4:name15:あいえおう12:p",
                    "iece lengthi536870912eee"),
                hashmap! {
                    "announce"      => B::Str("http://tracker.example.com:8080/announce"),
                    "comment"       => B::Str("\"Hello mock data\""),
                    "creation date" => B::Num(1234567890),
                    "httpseeds"     => B::List(vec![
                        B::Str("http://direct.example.com/mock1"),
                        B::Str("http://direct.example.com/mock2"),
                    ]),
                    "info" => B::Dict(hashmap!(
                        "length"       => B::Num(562949953421312),
                        "name"         => B::Str("あいえおう"),
                        "piece length" => B::Num(536870912),
                    )),
                }
            ),
        ];

        for (input, expected) in cases {
            let actual = B::nom_dict(input).unwrap().1;
            assert_eq!(actual, expected)
        }
    }

    #[test]
    fn parse_dict_fail() {
        let cases = vec!["d2:hi5:hello1:ai32ee"];

        for input in cases {
            assert!(B::nom_dict(input).is_err());
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
                ),
                [
                    0x83, 0x55, 0x11, 0x80, 0x8c,
                    0xd6, 0x54, 0x2c, 0x1b, 0xc5,
                    0x19, 0x8d, 0x2a, 0x48, 0x9d,
                    0xce, 0xd5, 0x2b, 0x53, 0x3a,
                ],
            ),
            (
                unsafe { from_utf8_unchecked(include_bytes!("test_data/mock_dir.torrent")) },
                [
                    0x74, 0x53, 0x68, 0x65, 0xe7,
                    0x7a, 0xcc, 0x72, 0xf2, 0x98,
                    0xc4, 0x88, 0xc3, 0x2c, 0x31,
                    0xab, 0x9b, 0x96, 0x98, 0xb1,
                ],
            ),
            (
                unsafe { from_utf8_unchecked(include_bytes!("test_data/mock_file.torrent")) },
                [
                    0x0b, 0x05, 0xab, 0xa1, 0xf2,
                    0xa0, 0xb2, 0xe6, 0xdc, 0x92,
                    0xf1, 0xdb, 0x11, 0x43, 0x3e,
                    0x5f, 0x3a, 0x82, 0x0b, 0xad,
                ],
            ),
        ];

        for (_torrent, _expected) in cases {
            // let info_hash = B::info_hash(torrent).unwrap();

            // assert_eq!(info_hash, expected);
        }
    }
}
