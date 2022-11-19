use std::collections::HashMap;

use nom::{
    branch::alt,
    bytes::complete::tag,
    character::complete::{char as nchar, digit0, digit1, one_of},
    combinator::{map, map_opt, opt, recognize},
    multi::{length_data, many0_count},
    sequence::{delimited, terminated, tuple},
};
use ring::digest;

use crate::utils::enum_conv;

// TorrentAST is a structural representation of a torrent file; fields map over almost identically,
// with dict's being represented as sub-structs
#[derive(Debug, PartialEq)]
pub struct TorrentAST<'a> {
    pub announce: &'a str,
    pub announce_list: Option<Vec<Vec<&'a str>>>,
    pub info: InfoAST<'a>,
}

#[derive(Debug, PartialEq)]
pub struct InfoAST<'a> {
    pub piece_length: i64,
    pub pieces: &'a [u8],
    pub private: Option<i64>,
    pub name: &'a str,

    // length and files are mutually exclusive
    pub length: Option<i64>,             // single file case
    pub files: Option<Vec<FileAST<'a>>>, // multi-file case
}

#[derive(Debug, PartialEq)]
pub struct FileAST<'a> {
    pub path: Vec<&'a str>,
    pub length: i64,
    pub attr: Option<&'a str>,
}

impl<'a> TorrentAST<'a> {
    pub fn decode(file: &[u8]) -> Option<TorrentAST> {
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
        match (&self.info.length, &self.info.files) {
            (Some(_), Some(_)) | (None, None) => return None,
            _ => (),
        };

        Some(self)
    }
}

impl FileAST<'_> {
    fn new(benc: Bencode) -> Option<FileAST> {
        let mut file = benc.dict()?;

        Some(FileAST {
            path: file.remove(&b"path"[..])?.map_list(|p| p.str())?,
            length: file.remove(&b"length"[..])?.num()?,
            attr: try { file.remove(&b"attr"[..])?.str()? },
        })
    }
}

#[derive(Debug, PartialEq, Clone)]
pub enum Bencode<'a> {
    Num(i64),
    Str(&'a [u8]),
    List(Vec<Bencode<'a>>),
    Dict(HashMap<&'a [u8], Bencode<'a>>),
}

enum_conv!(Bencode<'a>::Num, i64);
enum_conv!(Bencode<'a>::Str, &'a [u8]);
enum_conv!(Bencode<'a>::List, Vec<Bencode<'a>>);
enum_conv!(Bencode<'a>::Dict, HashMap<&'a [u8], Bencode<'a>>);

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
    pub fn decode(input: &[u8]) -> Option<Bencode> {
        // make sure we consumed the whole input
        let ([], benc) = Bencode::parse_benc(input).ok()? else {
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
    pub fn hash_dict(input: &[u8], key: &str) -> Option<[u8; 20]> {
        // SHA-1 hash includes surrounding 'd' and 'e' tags
        //
        // let input         = "d ... 4:infod ... e ... e";
        // let (start, end)  =     start -> [     ] <- end
        //
        // sha1.sum( input[start..=end] )
        let mut parse_kv_pair = tuple((Self::parse_str, Self::parse_benc_no_map));

        let mut input = nchar::<_, nom::error::Error<_>>('d')(input).ok()?.0;

        while let Ok((input_left, (k, val))) = parse_kv_pair(input) {
            if k == key.as_bytes() {
                return digest::digest(&digest::SHA1_FOR_LEGACY_USE_ONLY, val)
                    .as_ref()
                    .try_into()
                    .ok();
            }

            input = input_left;
        }

        None
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
    pub fn str(self) -> Option<&'a str> {
        std::str::from_utf8(self.bstr()?).ok()
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
    pub fn bstr(self) -> Option<&'a [u8]> {
        self.try_into().ok()
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
    pub fn num(self) -> Option<i64> {
        self.try_into().ok()
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
    pub fn list(self) -> Option<Vec<Bencode<'a>>> {
        self.try_into().ok()
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
    pub fn dict(self) -> Option<HashMap<&'a [u8], Bencode<'a>>> {
        self.try_into().ok()
    }

    /// map_list calls op with every element of a [Bencode::List], returning None if any call to
    /// op returned None
    ///
    /// # Examples
    /// ```ignore
    /// # use tsunami::torrent_ast::Bencode as B;
    ///
    /// let list = || vec![ B::Num(0), B::Num(1), B::Str(b"two") ];
    /// let benc = || B::List(list());
    ///
    /// assert!(benc().map_list(|b| Some(b)) == Some(list()));
    /// assert!(benc().map_list(|b| b.num()) == None);
    /// ```
    pub fn map_list<U>(self, op: impl Fn(Bencode<'a>) -> Option<U>) -> Option<Vec<U>> {
        self.list()?.into_iter().map(op).try_collect()
    }
}

type Parsed<'a, T> = nom::IResult<&'a [u8], T>;

impl<'a> Bencode<'a> {
    // nom bencode parsers

    fn parse_benc(input: &'a [u8]) -> Parsed<Bencode> {
        alt((
            map(Self::parse_str, Bencode::Str),
            map(Self::parse_int, Bencode::Num),
            map(Self::parse_list, Bencode::List),
            map(Self::parse_dict, Bencode::Dict),
        ))(input)
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
            |num| std::str::from_utf8(num).ok()?.parse().ok(),
        )(input)
    }

    // parse a valid bencoded list
    // pseudo format: l(Benc)*e
    fn parse_list(input: &'a [u8]) -> Parsed<Vec<Bencode>> {
        let mut input = nchar('l')(input)?.0;

        let mut list = vec![];
        while let Ok((input_left, benc)) = Self::parse_benc(input) {
            input = input_left;
            list.push(benc);
        }

        let input = nchar('e')(input)?.0;

        Ok((input, list))
    }

    // parse a valid bencoded dict
    // dict keys must appear in sorted order
    //
    // pseudo format: d(Str Benc)*e
    fn parse_dict(input: &'a [u8]) -> Parsed<HashMap<&[u8], Bencode>> {
        use nom::{
            error::{Error, ErrorKind},
            Err,
        };

        let mut parse_kv_pair = tuple((Self::parse_str, Self::parse_benc));

        let mut input = nchar('d')(input)?.0;
        let (mut dict, mut last_key) = (HashMap::new(), &b""[..]);

        while let Ok((input_left, (key, val))) = parse_kv_pair(input) {
            if last_key > key {
                return Err(Err::Error(Error::new(input, ErrorKind::MapOpt)));
            }
            (input, last_key) = (input_left, key);

            dict.insert(key, val);
        }

        let input = nchar('e')(input)?.0;

        Ok((input, dict))
    }

    // same as parse benc, but doesn't try to parse the resulting str's into Benc nodes
    // unfortunately we have to re-define all of the rules here :(
    fn parse_benc_no_map(input: &[u8]) -> Parsed<&[u8]> {
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
                many0_count(Self::parse_benc_no_map),
                nchar('e'),
            )),
            // dict
            recognize(delimited(
                nchar('d'),
                many0_count(tuple((Self::parse_str, Self::parse_benc_no_map))),
                nchar('e'),
            )),
        ))(input)
    }
}
