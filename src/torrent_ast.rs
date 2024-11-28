use std::{collections::HashMap, str::from_utf8_unchecked};

use sha1::{Digest, Sha1};

// TorrentAST is a structural representation of a torrent file; fields map over almost identically,
// with dict's being represented as sub-structs
#[derive(Debug, PartialEq)]
pub struct TorrentAST<'a> {
    pub announce: &'a str,
    pub announceList: Option<Vec<Vec<&'a str>>>,
    pub info: InfoAST<'a>,
}

#[derive(Debug, PartialEq)]
pub struct InfoAST<'a> {
    pub pieceLength: i64,
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
            announceList: try {
                torrent
                    .remove(&b"announce-list"[..])?
                    .map_list(|l| l.map_list(Bencode::str))?
            },
            info: InfoAST {
                name: info.remove(&b"name"[..])?.str()?,
                pieces: info.remove(&b"pieces"[..])?.bstr()?,
                pieceLength: info.remove(&b"piece length"[..])?.num()?,

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
        let mut tok = BencTokenizer {
            input,
            buildCollections: true,
        };
        let benc = tok.nextToken().ok()?;

        if !tok.input.is_empty() {
            return None;
        }

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
        if input.first() != Some(&b'd') {
            return None;
        }
        let mut tok = BencTokenizer {
            input: &input[1..],
            buildCollections: false,
        };

        loop {
            if tok.parseStr().ok()? == key.as_bytes() {
                // find length of dict, since nextToken advances input we have to get length in
                // a roundabout way.
                let dict = tok.input;
                tok.nextToken().ok()?;
                let dictLen = dict.len() - tok.input.len(); // whole slice - slice after nextToken() = bytes read

                return Some(Sha1::digest(&dict[..dictLen]).into());
            }

            tok.nextToken().ok()?;
        }
    }

    /// str unwraps a [Bencode::Str] variant
    ///
    /// # Examples
    /// ```ignore
    /// # use tsunami::torrent_ast::Bencode;
    ///
    /// assert!(Bencode::Str(b"str").str() == Some("str"));
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
    /// assert!(Bencode::Str(b"str").bstr() == Some(&b"str"[..]));
    /// ```
    pub fn bstr(self) -> Option<&'a [u8]> {
        let Bencode::Str(s) = self else { return None };
        Some(s)
    }

    /// num unwraps a [Bencode::Num] variant
    ///
    /// # Examples
    /// ```ignore
    /// # use tsunami::torrent_ast::Bencode;
    ///
    /// assert!(Bencode::Num(32).num() == Some(32));
    /// # assert!(Bencode::Str(b"str").num() == None);
    /// ```
    pub fn num(self) -> Option<i64> {
        let Bencode::Num(n) = self else { return None };
        Some(n)
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
    /// # assert!(Bencode::Str(b"str").list() == None);
    /// ```
    pub fn list(self) -> Option<Vec<Bencode<'a>>> {
        let Bencode::List(l) = self else { return None };
        Some(l)
    }

    /// dict unwraps a [Bencode::Dict] variant
    ///
    /// # Examples
    /// ```ignore
    /// # use std::collections::HashMap;
    /// # use tsunami::torrent_ast::Bencode;
    ///
    /// let dict = || { HashMap::from([ (&b"num"[..], Bencode::Num(32)) ]) };
    /// let benc = Bencode::Dict(dict());
    ///
    /// assert!(benc.dict() == Some(dict()));
    /// # assert!(Bencode::Str(b"str").dict() == None);
    /// ```
    pub fn dict(self) -> Option<HashMap<&'a [u8], Bencode<'a>>> {
        let Bencode::Dict(d) = self else { return None };
        Some(d)
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

#[derive(Debug, PartialEq)]
pub enum TokError {
    BencInvalidIdent,

    StrInvalidIdent,
    StrUnexpectedEOF,
    StrLenEOF,
    StrLenParseErr,
    StrEndOverflow,
    StrTooShort,
    StrInvalidSep,

    IntInvalidEncoding,
    IntInvalidIdent,
    IntUnexpectedEOF,
    IntLenEOF,
    IntExpectedEnd,
    IntLenParseErr,

    ListInvalidIdent,
    ListUnexpectedEOF,
    ListExpectedEnd,
    ListValParseErr,

    DictInvalidIdent,
    DictUnexpectedEOF,
    DictExpectedStrKey,
    DictKeysUnsorted,
    DictValParseErr,
    DictExpectedEnd,
}

pub struct BencTokenizer<'a> {
    pub input: &'a [u8],
    // disables building list and dicts. returned Vec/HashMap will always be empty
    pub buildCollections: bool,
}

impl<'a> BencTokenizer<'a> {
    fn nextToken(&mut self) -> Result<Bencode<'a>, TokError> {
        Ok(match self.input {
            [b'0'..=b'9', ..] => Bencode::Str(self.parseStr()?),
            [b'i', ..] => Bencode::Num(self.parseInt()?),
            [b'l', ..] => Bencode::List(self.parseList()?),
            [b'd', ..] => Bencode::Dict(self.parseDict()?),
            _ => return Err(TokError::BencInvalidIdent),
        })
    }

    /// parse a valid bencoded string
    ///
    /// a bencoded string is a base-ten length followed by a colon (:) and then the string
    ///
    /// # Examples
    /// ```ignore
    /// # use tsunami::torrent_ast::BencTokenizer;
    /// let mut tok = BencTokenizer::new("5:hello");
    /// assert!(tok.parseStr().unwrap() == &b"hello"[..]);
    /// ```
    pub fn parseStr(&mut self) -> Result<&'a [u8], TokError> {
        // check len starts with a non-zero digit to make parsing len simpler later
        match self.input {
            [b'1'..=b'9', ..] => (),
            [b'0', b':', rest @ ..] => {
                // empty fast path
                self.input = rest;
                return Ok(b"");
            }
            [_, ..] => return Err(TokError::StrInvalidIdent),
            [] => return Err(TokError::StrUnexpectedEOF),
        }

        // benc string: nnnnnnnn:cccccccccccccccccc
        //   len as u23 ^------^ ^--start    end--^
        let (start, end) = {
            let digitsEnd = self
                .input
                .iter()
                .position(|c| !(*c >= b'0' && *c <= b'9'))
                .ok_or(TokError::StrLenEOF)?;

            // limit strs to 2^32 bytes
            // SAFETY: we know input[..digitsEnd] only contains ASCII chars due to predicate fn
            //         used in position above
            // SAFETY: `as usize` should not overflow since we only support 32/64 bit systems
            let len = unsafe { from_utf8_unchecked(&self.input[..digitsEnd]) }
                .parse::<u32>()
                .map_err(|_| TokError::StrLenParseErr)? as usize;

            let start = digitsEnd + 1; // digitsEnd can be at most 10 chars (u32::MAX = 4294967295)
            let end = start.saturating_add(len);
            if end == usize::MAX {
                return Err(TokError::StrEndOverflow);
            }

            (start, end)
        };

        if self.input.len() < end {
            return Err(TokError::StrTooShort);
        }
        if self.input[start - 1] != b':' {
            return Err(TokError::StrInvalidSep);
        }

        let bstr = &self.input[start..end];
        self.input = &self.input[end..];

        Ok(bstr)
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
    pub fn parseInt(&mut self) -> Result<i64, TokError> {
        // skip leading negative sign and check if num starts with 1..=9 to simplify parsing later
        let skip = match self.input {
            [b'i', b'-', b'1'..=b'9', ..] => 2,
            [b'i', b'1'..=b'9', ..] => 1,
            [b'i', b'0', b'e', rest @ ..] => {
                // zero fast path
                self.input = rest;
                return Ok(0);
            }
            [b'i', ..] => return Err(TokError::IntInvalidEncoding),
            [_, ..] => return Err(TokError::IntInvalidIdent),
            [] => return Err(TokError::IntUnexpectedEOF),
        };

        let digitsEnd = self.input[skip..]
            .iter()
            .position(|c| !(*c >= b'0' && *c <= b'9'))
            .ok_or(TokError::IntLenEOF)?;

        let rest = match &self.input[digitsEnd + skip..] {
            [b'e', rest @ ..] => rest,
            _ => return Err(TokError::IntExpectedEnd),
        };

        // SAFETY: we know input[1..digitsEnd+skip] only contains ASCII chars due to predicate fn
        //         used in position above
        let num = unsafe { from_utf8_unchecked(&self.input[1..digitsEnd + skip]) }
            .parse::<i64>()
            .map_err(|_| TokError::IntLenParseErr)?;

        self.input = rest;

        Ok(num)
    }

    // parse a valid bencoded list
    // pseudo format: l(Benc)*e
    pub fn parseList(&mut self) -> Result<Vec<Bencode<'a>>, TokError> {
        let mut list = vec![];

        match self.input {
            [b'l', b'e', rest @ ..] => {
                // empty fast path
                self.input = rest;
                return Ok(list);
            }
            [b'l', ..] => self.input = &self.input[1..],
            [_, ..] => return Err(TokError::ListInvalidIdent),
            [] => return Err(TokError::ListUnexpectedEOF),
        }

        loop {
            match self.nextToken() {
                Ok(tok) if self.buildCollections => list.push(tok),
                Ok(_) => (),
                Err(TokError::BencInvalidIdent) => break,
                Err(_) => return Err(TokError::ListValParseErr),
            }
        }

        match self.input {
            [b'e', rest @ ..] => self.input = rest,
            _ => return Err(TokError::ListExpectedEnd),
        }

        Ok(list)
    }

    // parse a valid bencoded dict
    // dict keys must appear in sorted order
    //
    // pseudo format: d(Str Benc)*e
    pub fn parseDict(&mut self) -> Result<HashMap<&'a [u8], Bencode<'a>>, TokError> {
        let mut dict = HashMap::new();
        let mut prevKey = &self.input[..0];

        match self.input {
            [b'd', b'e', rest @ ..] => {
                // empty fast path
                self.input = rest;
                return Ok(dict);
            }
            [b'd', ..] => self.input = &self.input[1..],
            [_, ..] => return Err(TokError::DictInvalidIdent),
            [] => return Err(TokError::DictUnexpectedEOF),
        }

        loop {
            let k = match self.parseStr() {
                Ok(k) => k,
                Err(TokError::StrInvalidIdent) => break,
                Err(_) => return Err(TokError::DictExpectedStrKey),
            };

            if k < prevKey {
                return Err(TokError::DictKeysUnsorted);
            }
            prevKey = k;

            match self.nextToken() {
                Ok(v) if self.buildCollections => dict.insert(k, v),
                Ok(_) => None,
                Err(_) => return Err(TokError::DictValParseErr),
            };
        }

        match self.input {
            [b'e', rest @ ..] => self.input = rest,
            _ => return Err(TokError::DictExpectedEnd),
        }

        Ok(dict)
    }
}
