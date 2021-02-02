use lalrpop_util::{lalrpop_mod, ParseError};
use logos::{Lexer, Logos};
use std::collections::HashMap;
use std::str::from_utf8;

type Spanned<Tok, Loc, Error> = Result<(Loc, Tok, Loc), Error>;
type DecodeResult<'a> = Result<Bencode<'a>, ParseError<usize, Token<'a>, BencError>>;

#[derive(Debug, PartialEq)]
pub enum Bencode<'a> {
    Num(i64),
    Str(&'a str),
    BStr(&'a [u8]),
    List(Vec<Bencode<'a>>),
    Dict(HashMap<&'a str, Bencode<'a>>),
}

impl<'a> Bencode<'a> {
    pub fn decode(input: &str) -> DecodeResult {
        let parser = bencode_lexer::BencParser::new();

        let lex = Token::lexer(input);
        parser.parse(input, lex)
    }

    pub fn decode_num(sign: Option<Token>, n: i64) -> DecodeResult {
        if sign.is_some() && n == 0 {
            return Err(ParseError::User {
                error: BencError::NegativeZero,
            });
        }

        // todo: -i64::MAX => valid?
        Ok(Bencode::Num(if sign.is_some() { -n } else { n }))
    }

    pub fn decode_str(input: &[u8]) -> Bencode {
        from_utf8(input).map_or(Bencode::BStr(input), |s| Bencode::Str(s))
    }

    pub fn decode_dict(list: Vec<(&'a [u8], Bencode<'a>)>) -> DecodeResult<'a> {
        // check keys are in sorted order
        if !list[..].windows(2).all(|w| w[0].0 < w[1].0) {
            return Err(ParseError::User {
                error: BencError::DictKeysNotSorted,
            });
        }

        // TODO - what if keys are not utf-8?
        let mut dict = HashMap::new();

        let dict_tuples = list.into_iter().map(|t| (from_utf8(t.0).ok(), t.1));
        for (k, v) in dict_tuples {
            match k {
                Some(k) => dict.insert(k, v),
                None => {
                    return Err(ParseError::User {
                        error: BencError::ParseError,
                    })
                }
            };
        }

        Ok(Bencode::Dict(dict))
    }

    pub fn str(self) -> Option<&'a str> {
        match self {
            Bencode::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn byte_str(self) -> Option<&'a [u8]> {
        match self {
            Bencode::BStr(b) => Some(b),
            Bencode::Str(s) => Some(s.as_bytes()),
            _ => None,
        }
    }

    pub fn num(self) -> Option<i64> {
        match self {
            Bencode::Num(n) => Some(n),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn list(self) -> Option<Vec<Bencode<'a>>> {
        match self {
            Bencode::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn dict(self) -> Option<HashMap<&'a str, Bencode<'a>>> {
        match self {
            Bencode::Dict(d) => Some(d),
            _ => None,
        }
    }

    pub fn map_list<U>(self, f: impl Fn(Bencode<'a>) -> Option<U>) -> Option<Vec<U>> {
        match self {
            Bencode::List(l) => {
                let mut v = Vec::with_capacity(l.len());
                for b in l {
                    v.push(f(b)?);
                }

                Some(v)
            }
            _ => None,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum BencError {
    NegativeZero,
    DictKeysNotSorted,
    ParseError,
}

lalrpop_mod!(pub bencode_lexer);

#[derive(Debug, Clone, PartialEq, Logos)]
pub enum Token<'a> {
    #[token("i")]
    I,
    #[token("l")]
    L,
    #[token("d")]
    D,
    #[token("e")]
    E,
    #[token(":")]
    Colon,
    #[token("-")]
    Minus,

    #[token("0", |_| 0)]
    #[regex("[1-9][0-9]*", |lex| lex.slice().parse())]
    Num(i64),

    #[regex("[0-9]+:", Token::lex_str)]
    Str((u32, &'a [u8])), // TODO - remove length?

    #[error]
    Error,
}

impl<'a> Token<'a> {
    fn lexer(input: &str) -> impl Iterator<Item = Spanned<Token, usize, BencError>> {
        <Token as Logos>::lexer(input)
            .spanned()
            .map(|(tok, span)| match tok {
                Token::Error => Err(BencError::ParseError),
                _ => Ok((span.start, tok, span.end)),
            })
    }

    fn lex_str(lex: &mut Lexer<'a, Token<'a>>) -> Option<(u32, &'a [u8])> {
        let len_slice = lex.slice();
        let len = len_slice[..len_slice.len() - 1].parse::<u32>().ok()?;
        let remainder = lex.remainder();

        if remainder.len() >= len as usize {
            let str = &lex.remainder()[..len as usize];
            lex.bump(len as usize);

            Some((len, str.as_bytes()))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{bencode_lexer, Bencode as B, Token};
    use std::collections::HashMap;

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
            ("i9223372036854775807e", 9223372036854775807),
        ];

        lex_tests_helper(cases, |n| B::Num(n));
    }

    #[test]
    fn parse_int_fail() {
        let cases = vec![
            "e",
            "-0e",
            "00e",
            "05e",
            "i18446744073709551615e",
            "i-0e",
            "i03e",
        ];

        lex_fail_tests_helper(cases);
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

        lex_tests_helper(cases, |s| B::Str(s));
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

        lex_fail_tests_helper(cases);
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
                            "list"    => B::List(vec!(B::Num(1), B::Num(2), B::Num(3))),
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

        lex_tests_helper(cases, |l| B::List(l));
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
                    "httpseeds"     => B::List(vec!(
                        B::Str("http://direct.example.com/mock1"),
                        B::Str("http://direct.example.com/mock2"),
                    )),
                    "info" => B::Dict(hashmap!(
                        "length"       => B::Num(562949953421312),
                        "name"         => B::Str("あいえおう"),
                        "piece length" => B::Num(536870912),
                    )),
                }
            ),
        ];

        lex_tests_helper(cases, |d| B::Dict(d));
    }

    #[test]
    fn parse_dict_fail() {
        let cases = vec!["d2:hi5:hello1:ai32ee"];

        lex_fail_tests_helper(cases);
    }

    fn lex_tests_helper<T>(cases: Vec<(&str, T)>, f: impl Fn(T) -> B<'static>) {
        let parser = bencode_lexer::BencParser::new();

        for (input, expected) in cases {
            let lex = Token::lexer(input);
            let res = parser.parse(input, lex);

            assert_eq!(res, Ok(f(expected)));
        }
    }

    fn lex_fail_tests_helper(cases: Vec<&str>) {
        let parser = bencode_lexer::BencParser::new();

        for input in cases {
            let lex = Token::lexer(input);
            let res = parser.parse(input, lex);

            assert!(res.is_err());
        }
    }
}
