use std::{collections::HashMap, ffi::OsStr, iter::once, path::PathBuf};

use crate::bencode::Bencode;

/// Torrent keeps a torrents metadata in a more workable format
#[derive(Debug, PartialEq)]
pub struct Torrent {
    // announce_list contains a group of one or more trackers followed by an
    // optional list of backup groups. this will always contain at least one
    // tracker (`announce_list[0][0]`)
    //
    // example: vec![ vec!["tracker1", "tr2"], vec!["backup1"] ]
    pub trackers_list: Vec<Vec<String>>,
    pub info: Info,
    pub info_hash: [u8; 20],
}

#[derive(Debug, PartialEq)]
pub struct Info {
    piece_length: u32,
    pieces: Vec<[u8; 20]>,
    private: bool,

    pub files: Vec<File>,
}

#[derive(Debug, PartialEq)]
pub struct File {
    // absolute location where file is saved. By default this is usually `OS_DOWNLOAD_DIR + base_path`
    file: PathBuf,

    // relative path as defined in the torrent file. this is may be sanitized for OS-specific
    // character limitations or other blacklisted file names. since this is purely advisory, file
    // may differ from base_path
    // todo: do we need to keep base_path around if we already have file
    base_path: PathBuf,

    pub length: u64,
}

impl Torrent {
    pub fn decode(torrent_file: &[u8]) -> Option<Torrent> {
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
            Some(ref lss) => lss
                .iter()
                .map(|ls| ls.iter().map(|l| (*l).into()).collect())
                .collect(),
            None => vec![vec![torrent.announce.into()]],
        };

        Some(Torrent {
            trackers_list: announce_list,

            info: Info {
                piece_length: torrent.info.piece_length.try_into().ok()?,
                pieces,
                private: torrent.info.private == Some(1),
                files: Self::build_files(&torrent.info)?,
            },
            info_hash: Bencode::dict_hash(torrent_file, "info")?,
        })
    }

    fn build_files(info: &InfoAST) -> Option<Vec<File>> {
        // for single file torrents this is `InfoAST.name`
        // for  multi file torrents this is `InfoAST.name + FileAST.path.join("/")`

        // todo: should we check for invalid paths? (incl os-specific blacklists) ?
        // todo: name is advisory, can we just name it anything? should we change base_path if we
        //       have an os_path member with the actual location on disk?
        // todo: should we backtrack '..' up one dir?
        let valid_path = |p: &str| p != "." && p != "..";

        // fixme: move this to file_asts match block where it's used. have to define this here since
        // "borrowed value does not live long enough"
        let mut tmp = [FileAST {
            path: Vec::with_capacity(1),
            length: 0,
        }];

        let (base_dir, file_asts) = match (&info.length, &info.files) {
            // todo: info.name could be "", making all files here top-level
            (None, Some(ref files)) => (info.name, &files[..]),
            (Some(len), None) => {
                tmp[0].path.push(info.name);
                tmp[0].length = *len;

                ("", &tmp[..])
            }
            _ => return None,
        };

        let mut files = vec![];
        for file in file_asts {
            // file must be non-zero bytes
            if file.length <= 0 {
                return None;
            }

            let parts = file.path.iter().filter(|p| valid_path(p));
            let base_path = PathBuf::from_iter(once(&base_dir).chain(parts));

            // path was empty or all path segments were filtered out
            if base_path == OsStr::new(base_dir) {
                return None;
            }

            files.push(File {
                // todo: file should be an absolute path
                file: base_path.clone(),
                base_path,
                length: file.length as u64,
            });
        }

        Some(files)
    }
}

// TorrentAST is a structural representation of a torrent file; fields map over almost identically,
// with dict's being represented as sub-structs
#[derive(Debug, PartialEq)]
struct TorrentAST<'a> {
    announce: &'a str,
    announce_list: Option<Vec<Vec<&'a str>>>,
    info: InfoAST<'a>,

    comment: Option<&'a str>,
    created_by: Option<&'a str>,
    creation_date: Option<i64>,
    encoding: Option<&'a str>,
}

#[derive(Debug, PartialEq)]
struct InfoAST<'a> {
    piece_length: i64,
    pieces: &'a [u8],
    private: Option<i64>,
    name: &'a str,

    // length and files are mutually exclusive
    // single file case
    length: Option<i64>,
    // multi-file case
    files: Option<Vec<FileAST<'a>>>,
}

#[derive(Debug, PartialEq)]
struct FileAST<'a> {
    path: Vec<&'a str>,
    length: i64,
}

impl<'a> TorrentAST<'a> {
    fn decode(file: &'a [u8]) -> Option<TorrentAST<'a>> {
        let mut torrent = Bencode::decode(file)?.dict()?;
        let mut info = torrent.remove("info")?.dict()?;

        Some(TorrentAST {
            announce: torrent.remove("announce")?.str()?,

            announce_list: torrent
                .remove("announce-list")
                .and_then(|al| al.map_list(|l| l.map_list(Bencode::str))),

            info: InfoAST {
                piece_length: info.remove("piece length")?.num()?,
                pieces: info.remove("pieces")?.bstr()?,
                private: info.remove("private").and_then(Bencode::num),
                name: info.remove("name")?.str()?,
                length: info.remove("length").and_then(Bencode::num),
                files: info
                    .remove("files")
                    .and_then(|f| f.map_list(|b| Self::decode_file(b.dict()?))),
            },

            comment: torrent.remove("comment").and_then(Bencode::str),
            created_by: torrent.remove("created by").and_then(Bencode::str),
            creation_date: torrent.remove("creation date").and_then(Bencode::num),
            encoding: torrent.remove("encoding").and_then(Bencode::str),
        })
    }

    fn decode_file(mut file: HashMap<&'a str, Bencode<'a>>) -> Option<FileAST<'a>> {
        Some(FileAST {
            path: file.remove("path")?.map_list(|p| p.str())?,
            length: file.remove("length")?.num()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{iter::once, path::PathBuf};

    use crate::torrent::{File, Info, Torrent};

    #[test]
    fn decode_torrent() {
        let tor_gen = |prefix: &str| Torrent {
            trackers_list: vec![
                vec!["http://tracker.example.com".into()],
                vec!["http://tracker2.example.com".into()],
            ],
            info: Info {
                piece_length: 32768,
                pieces: vec![[
                    0, 72, 105, 249, 236, 50, 141, 28, 177, 230, 77, 80, 106, 67, 249, 35, 207,
                    173, 235, 151,
                ]],
                private: true,
                files: vec![File {
                    base_path: PathBuf::from_iter(once(prefix).chain(once("file.txt"))),
                    file: PathBuf::from_iter(once(prefix).chain(once("file.txt"))),
                    length: 10,
                }],
            },
            info_hash: if prefix == "" {
                [
                    11, 5, 171, 161, 242, 160, 178, 230, 220, 146, 241, 219, 17, 67, 62, 95, 58,
                    130, 11, 173,
                ]
            } else {
                [
                    116, 83, 104, 101, 231, 122, 204, 114, 242, 152, 196, 136, 195, 44, 49, 171,
                    155, 150, 152, 177,
                ]
            },
        };

        let test_files = [
            (&include_bytes!("test_data/mock_dir.torrent")[..], "mock"),
            (&include_bytes!("test_data/mock_file.torrent")[..], ""),
        ];

        for (file, dir_name) in test_files {
            let torrent = Torrent::decode(file).unwrap();
            assert_eq!(torrent, tor_gen(dir_name));
        }
    }
}
