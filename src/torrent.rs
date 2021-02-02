use crate::bencode::Bencode;
use std::collections::HashMap;
use std::convert::TryFrom;
use std::vec;

// TODO - transformation pipelines ==>
//   RawTorrent (decoded to struct repr of bytes) ->
//   Torrent (dev friendly to work with)
#[derive(Debug, PartialEq)]
struct RawTorrent<'a> {
    announce: &'a str,
    announce_list: Option<Vec<Vec<&'a str>>>,
    comment: Option<&'a str>,
    created_by: Option<&'a str>,
    creation_date: Option<i64>,
    encoding: Option<&'a str>,

    info: RawInfo<'a>,
}

#[derive(Debug, PartialEq)]
struct RawInfo<'a> {
    piece_length: i64,
    pieces: &'a [u8],
    private: Option<i64>,
    name: &'a str,

    // single file
    length: Option<i64>,
    md5sum: Option<&'a str>,

    // multi-file
    files: Option<Vec<RawFile<'a>>>,
}

#[derive(Debug, PartialEq)]
struct RawFile<'a> {
    path: Vec<&'a str>,
    length: i64,
    md5sum: Option<&'a str>,
}

impl<'a> RawTorrent<'a> {
    fn decode(file: &'a str) -> Option<RawTorrent<'a>> {
        let benc = Bencode::decode(file).ok()?;

        let mut torrent = benc.dict()?;
        let mut info = torrent.remove("info")?.dict()?;

        Some(RawTorrent {
            announce: torrent.remove("announce").and_then(Bencode::str)?,
            announce_list: torrent
                .remove("announce-list")
                .and_then(|al| al.map_list(|l| l.map_list(Bencode::str))),
            comment: torrent.remove("comment").and_then(Bencode::str),
            created_by: torrent.remove("created by").and_then(Bencode::str),
            creation_date: torrent.remove("creation date").and_then(Bencode::num),
            encoding: torrent.remove("encoding").and_then(Bencode::str),
            info: RawInfo {
                piece_length: info.remove("piece length")?.num()?,
                pieces: info.remove("pieces")?.byte_str()?,
                private: info.remove("private")?.num(),
                name: info.remove("name")?.str()?,
                length: info.remove("length").and_then(Bencode::num),
                md5sum: info.remove("md5sum").and_then(Bencode::str),
                files: info
                    .remove("files")
                    .and_then(|f| f.map_list(|b| RawFile::decode(b.dict()?))),
            },
        })
    }
}

impl<'a> RawInfo<'a> {
    fn is_multi_file(&self) -> bool {
        self.length.is_some()
    }
}

impl<'a> RawFile<'a> {
    fn decode(mut file: HashMap<&'a str, Bencode<'a>>) -> Option<RawFile<'a>> {
        Some(RawFile {
            path: file.remove("path")?.map_list(|p| p.str())?,
            length: file.remove("length")?.num()?,
            md5sum: file.remove("md5sum").and_then(|s| s.str()),
        })
    }
}

#[derive(Debug, PartialEq)]
pub struct Torrent<'a> {
    announce_list: Vec<Vec<&'a str>>,
    info: Info<'a>,
}

#[derive(Debug, PartialEq)]
pub struct Info<'a> {
    piece_length: u32,
    pieces: &'a [u8],
    private: bool,

    dir_name: &'a str, // "" == single file (files.len() == 1)
    files: Vec<File<'a>>,
}

#[derive(Debug, PartialEq)]
pub struct File<'a> {
    // looks like torrent files don't contain empty folders or files
    path: Vec<&'a str>, // path.concat("/")
    length: u64,
    md5sum: Option<&'a str>,
}

impl<'a> Torrent<'a> {
    pub fn decode(torrent_file: &'a str) -> Option<Torrent> {
        let torrent = RawTorrent::decode(torrent_file)?;

        Some(Torrent {
            announce_list: torrent
                .announce_list
                .unwrap_or(vec![vec![torrent.announce]]),

            info: Info {
                piece_length: u32::try_from(torrent.info.piece_length).ok()?,
                pieces: torrent.info.pieces,
                private: torrent.info.private == Some(1),
                dir_name: if torrent.info.is_multi_file() {
                    ""
                } else {
                    torrent.info.name
                },
                files: Self::build_files(torrent.info)?,
            },
        })
    }

    fn build_files(info: RawInfo<'a>) -> Option<Vec<File<'a>>> {
        if info.is_multi_file() {
            // single file case
            Some(vec![File {
                path: vec![info.name],
                length: info.length? as u64,
                md5sum: info.md5sum,
            }])
        } else {
            // multi file case
            Some(info.files?.into_iter().map(File::from).collect())
        }
    }
}

impl<'a> From<RawFile<'a>> for File<'a> {
    fn from(rf: RawFile<'a>) -> Self {
        File {
            path: rf.path,
            length: rf.length as u64, // todo: fails if length < 0
            md5sum: rf.md5sum,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Torrent;
    use std::str::from_utf8_unchecked;

    #[test]
    fn decode_torrent() {
        let file = unsafe { from_utf8_unchecked(include_bytes!("test_data/mock_dir.torrent")) };
        println!("{:#?}", Torrent::decode(file).unwrap());

        let file = unsafe { from_utf8_unchecked(include_bytes!("test_data/mock_file.torrent")) };
        println!("{:#?}", Torrent::decode(file).unwrap());

        assert!(false);
    }
}
