use crate::bencode::Bencode;
use crate::utils::IterExt;
use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::vec;

#[derive(Debug, PartialEq)]
pub struct Torrent {
    announce_list: Vec<Vec<String>>,
    info: Info,
    info_hash: [u8; 20],
}

#[derive(Debug, PartialEq)]
pub struct Info {
    piece_length: u32,
    pieces: Vec<[u8; 20]>,
    private: bool,

    dir_name: String, // "" == single file (files.len() == 1)
    files: Vec<File>,
}

#[derive(Debug, PartialEq)]
pub struct File {
    // looks like torrent files don't contain empty folders or files
    path: Vec<String>, // path.concat("/")
    length: u64,
    md5sum: Option<String>,
}

impl Torrent {
    pub fn decode(torrent_file: &str) -> Option<Torrent> {
        let torrent = TorrentAST::decode(torrent_file)?;

        if torrent.info.pieces.len() % 20 != 0 {
            return None;
        }

        let pieces = torrent
            .info
            .pieces
            .chunks(20)
            .map(|p| p.try_into().unwrap())
            .collect();

        let announce_list = match torrent.announce_list {
            Some(lss) => lss
                .into_iter()
                .map(|ls| ls.into_iter().map(|l| l.into()).collect())
                .collect(),
            None => vec![vec![torrent.announce.into()]],
        };

        Some(Torrent {
            announce_list: announce_list,

            info: Info {
                piece_length: torrent.info.piece_length.try_into().ok()?,
                pieces: pieces,
                private: torrent.info.private == Some(1),
                dir_name: if torrent.info.single_file() {
                    "".into()
                } else {
                    torrent.info.name.into()
                },
                files: Self::build_files(torrent.info)?,
            },
            info_hash: Bencode::info_hash(torrent_file)?,
        })
    }

    fn build_files(info: InfoAST) -> Option<Vec<File>> {
        if info.single_file() {
            // single file case
            Some(vec![File {
                path: vec![info.name.into()],
                length: info.length? as u64,
                md5sum: info.md5sum.map(|m| m.into()),
            }])
        } else {
            info.files?.into_iter().flat_map_all(|f| f.try_into().ok())
        }
    }
}

impl TryFrom<FileAST<'_>> for File {
    type Error = ();

    fn try_from(rf: FileAST) -> Result<Self, Self::Error> {
        Ok(File {
            path: rf.path.into_iter().map(|p| p.into()).collect(),
            length: rf.length.try_into().map_err(|_| ())?, // negative lengths are invalid
            md5sum: rf.md5sum.map(|m| m.into()),
        })
    }
}

#[derive(Debug, PartialEq)]
struct TorrentAST<'a> {
    announce: &'a str,
    announce_list: Option<Vec<Vec<&'a str>>>,
    comment: Option<&'a str>,
    created_by: Option<&'a str>,
    creation_date: Option<i64>,
    encoding: Option<&'a str>,

    info: InfoAST<'a>,
}

#[derive(Debug, PartialEq)]
struct InfoAST<'a> {
    piece_length: i64,
    pieces: &'a [u8],
    private: Option<i64>,
    name: &'a str,

    // single file
    length: Option<i64>,
    md5sum: Option<&'a str>,

    // multi-file
    files: Option<Vec<FileAST<'a>>>,
}

#[derive(Debug, PartialEq)]
struct FileAST<'a> {
    path: Vec<&'a str>,
    length: i64,
    md5sum: Option<&'a str>,
}

impl<'a> TorrentAST<'a> {
    fn decode(file: &'a str) -> Option<TorrentAST<'a>> {
        let benc = Bencode::decode(file).ok()?;

        let mut torrent = benc.dict()?;
        let mut info = torrent.remove("info")?.dict()?;

        Some(TorrentAST {
            announce: torrent.remove("announce").and_then(Bencode::str)?,
            announce_list: torrent
                .remove("announce-list")
                .and_then(|al| al.map_list(|l| l.map_list(Bencode::str))),
            comment: torrent.remove("comment").and_then(Bencode::str),
            created_by: torrent.remove("created by").and_then(Bencode::str),
            creation_date: torrent.remove("creation date").and_then(Bencode::num),
            encoding: torrent.remove("encoding").and_then(Bencode::str),
            info: InfoAST {
                piece_length: info.remove("piece length")?.num()?,
                pieces: info.remove("pieces")?.str()?.as_bytes(),
                private: info.remove("private")?.num(),
                name: info.remove("name")?.str()?,
                length: info.remove("length").and_then(Bencode::num),
                md5sum: info.remove("md5sum").and_then(Bencode::str),
                files: info
                    .remove("files")
                    .and_then(|f| f.map_list(|b| FileAST::decode(b.dict()?))),
            },
        })
    }
}

impl InfoAST<'_> {
    fn single_file(&self) -> bool {
        self.length.is_some()
    }
}

impl<'a> FileAST<'a> {
    fn decode(mut file: HashMap<&'a str, Bencode<'a>>) -> Option<FileAST<'a>> {
        Some(FileAST {
            path: file.remove("path")?.map_list(|p| p.str())?,
            length: file.remove("length")?.num()?,
            md5sum: file.remove("md5sum").and_then(|s| s.str()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Torrent;
    use std::str::from_utf8_unchecked;

    #[test]
    fn decode_torrent() {
        let test_files = [
            (
                unsafe { from_utf8_unchecked(include_bytes!("test_data/mock_dir.torrent")) },
                "mock",
            ),
            (
                unsafe { from_utf8_unchecked(include_bytes!("test_data/mock_file.torrent")) },
                "",
            ),
        ];

        for (file, dir_name) in &test_files[..] {
            let pieces: Vec<[u8; 20]> = vec![[
                0, 72, 105, 249, 236, 50, 141, 28, 177, 230, 77, 80, 106, 67, 249, 35, 207, 173,
                235, 151,
            ]];

            let torrent = Torrent::decode(file).unwrap();
            println!("{:?}", torrent);

            assert_eq!(torrent.info.dir_name, *dir_name);
            assert_eq!(torrent.info.pieces, pieces);
        }
    }
}
