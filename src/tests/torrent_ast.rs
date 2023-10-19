use std::{cmp::min, collections::HashMap};

use crate::{
    tests::test_data,
    torrent_ast::{Bencode as B, Bencode},
};

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
        let actual = B::decode(input.as_bytes()).unwrap().num().unwrap();
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
        assert!(B::decode(input.as_bytes()).is_none());
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
        let actual = B::decode(input.as_bytes()).unwrap().str().unwrap();
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
        assert!(B::decode(input.as_bytes()).is_none());
    }
}

#[test]
fn parse_list() {
    let cases = vec![
        ("le", vec![]),
        ("li4ei2e2:42e", vec![B::Num(4), B::Num(2), B::Str(b"42")]),
        (
            "l5:helloi42eli2ei3e2:hid4:listli1ei2ei3ee7:yahallo2::)eed2:hi5:hello3:inti15eee",
            vec![
                B::Str(b"hello"),
                B::Num(42),
                B::List(vec![
                    B::Num(2),
                    B::Num(3),
                    B::Str(b"hi"),
                    B::Dict(hashmap! {
                        &b"list"[..]    => B::List(vec![B::Num(1), B::Num(2), B::Num(3)]),
                        &b"yahallo"[..] => B::Str(b":)"),
                    }),
                ]),
                B::Dict(hashmap! {
                    &b"hi"[..]  => B::Str(b"hello"),
                    &b"int"[..] => B::Num(15),
                }),
            ],
        ),
    ];

    for (input, expected) in cases {
        let actual = B::decode(input.as_bytes()).unwrap().list().unwrap();
        assert_eq!(actual, expected)
    }
}

#[test]
fn parse_dict() {
    let cases = vec![
        ("de", HashMap::new()),
        (
            "d3:onei1e3:twoi2ee",
            hashmap! { &b"one"[..] => B::Num(1), &b"two"[..] => B::Num(2) },
        ),
        (
            concat!(
                "d8:announce40:http://tracker.example.com:8080/announce7:comment17:\"Hello mock data",
                "\"13:creation datei1234567890e9:httpseedsl31:http://direct.example.com/mock131:http",
                "://direct.example.com/mock2e4:infod6:lengthi562949953421312e4:name15:あいえおう12:p",
                "iece lengthi536870912eee"
            ),
            hashmap! {
                &b"announce"[..]      => B::Str(b"http://tracker.example.com:8080/announce"),
                &b"comment"[..]       => B::Str(b"\"Hello mock data\""),
                &b"creation date"[..] => B::Num(1234567890),
                &b"httpseeds"[..]     => B::List(vec![
                    B::Str(b"http://direct.example.com/mock1"),
                    B::Str(b"http://direct.example.com/mock2"),
                ]),
                &b"info"[..] => B::Dict(hashmap!{
                    &b"length"[..]       => B::Num(562949953421312),
                    &b"name"[..]         => B::Str(b"\xE3\x81\x82\xE3\x81\x84\xE3\x81\x88\xE3\x81\x8A\xE3\x81\x86"),
                    &b"piece length"[..] => B::Num(536870912),
                }),
            },
        ),
    ];

    for (input, expected) in cases {
        let actual = B::decode(input.as_bytes()).unwrap().dict().unwrap();
        assert_eq!(actual, expected)
    }
}

#[test]
fn parse_dict_fail() {
    let cases = vec!["d2:hi5:hello1:ai32ee"];

    for input in cases {
        assert!(B::decode(input.as_bytes()).is_none());
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
            )
            .as_bytes(),
            [
                0x83, 0x55, 0x11, 0x80, 0x8c, 0xd6, 0x54, 0x2c, 0x1b, 0xc5, 0x19, 0x8d, 0x2a, 0x48,
                0x9d, 0xce, 0xd5, 0x2b, 0x53, 0x3a,
            ],
        ),
        (
            test_data::MOCK_DIR,
            [
                0x74, 0x53, 0x68, 0x65, 0xe7, 0x7a, 0xcc, 0x72, 0xf2, 0x98, 0xc4, 0x88, 0xc3, 0x2c,
                0x31, 0xab, 0x9b, 0x96, 0x98, 0xb1,
            ],
        ),
        (
            test_data::MOCK_FILE,
            [
                0x0b, 0x05, 0xab, 0xa1, 0xf2, 0xa0, 0xb2, 0xe6, 0xdc, 0x92, 0xf1, 0xdb, 0x11, 0x43,
                0x3e, 0x5f, 0x3a, 0x82, 0x0b, 0xad,
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
        //
        test_data::BTV2_TEST,
        test_data::BTV2_HYBRID_TEST,
    ];

    for file in test_files {
        let torrent = B::decode(file).unwrap();
        print_benc(torrent, 2);
    }
}

fn print_benc(v: Bencode, spaces: usize) {
    match v {
        Bencode::Num(_) => {
            print!("{v:?},")
        }
        Bencode::Str(b) => {
            if let Ok(s) = std::str::from_utf8(b) {
                print!("{s:?}");
            } else {
                let trunc = b.split_at(min(20, b.len())).0;
                print!("{trunc:?}");
            }
        }
        Bencode::List(l) => {
            // if l.len() < 4 {
            //     print!("{l:?},");
            //     return;
            // }

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
                (0..spaces).for_each(|_| print!(" "));

                if let Ok(s) = std::str::from_utf8(k) {
                    print!("{s:?}: ");
                } else {
                    let trunc = k.split_at(min(20, k.len())).0;
                    print!("{trunc:?}");
                }

                // let k = String::from_utf8_lossy(k);
                // print!("{k:?} => ");
                // println!("`dbg: {v:?}`");
                print_benc(v, spaces + 2);
                println!();
            }

            (0..spaces - 2).for_each(|_| print!(" "));
            print!("}}");
        }
    }
}
