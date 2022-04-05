use std::{
    collections::HashMap,
    ffi::OsStr,
    fmt::Write,
    net::{Ipv4Addr, SocketAddrV4},
    path::{Path, PathBuf},
    sync::Arc,
};

use byteorder::{ByteOrder, BE};
use chrono::{DateTime, Duration, Utc};
use hyper::{body, client::HttpConnector, Client};
use rand::{rngs::SmallRng, seq::SliceRandom, SeedableRng};

use crate::{
    error::{Error, Result},
    peer::Peer,
    torrent_ast::{Bencode, InfoAST, TorrentAST},
};

pub type Sha1Hash = [u8; 20];

/// Torrent keeps a torrents metadata in a more workable format
#[derive(Debug)]
pub struct Torrent {
    info: Info,
    peers: HashMap<SocketAddrV4, Option<Peer>>,

    // trackers is a group of one or more trackers followed by an optional list of backup groups.
    // this will always contain at least one tracker (`announce_list[0][0]`)
    //
    // example: vec![ vec!["tracker1", "tr2"], vec!["backup1"] ]
    trackers: Vec<Vec<String>>,
    next_announce: DateTime<Utc>,

    peer_id: Arc<String>,
    bytes_left: u64,
    uploaded: u64,
    downloaded: u64,
}

#[derive(Debug, PartialEq)]
struct Info {
    files: Vec<File>,

    piece_length: u32,
    pieces: Vec<Sha1Hash>,
    info_hash: Sha1Hash,

    private: bool,
}

#[derive(Debug, PartialEq)]
struct File {
    // absolute location where file is saved. this defaults to base_path, but may be sanitized for
    // OS-specific character limitations or other malformed file names
    // default: OS_DOWNLOAD_DIR | HOME + base_path
    file: PathBuf,
    length: u64,
}

impl Torrent {
    pub fn new(buf: &[u8], peer_id: Arc<String>, base_dir: &Path) -> Option<Torrent> {
        let torrent = TorrentAST::decode(buf)?;
        let info = torrent.info;

        let pieces = {
            // if let num_pieces = info.pieces.len() &&
            //     num_pieces % 20 != 0 || num_pieces > 1 << 32 { return None; }

            // pieces is a list of 20 byte sha1 hashes
            if info.pieces.len() % 20 != 0 {
                return None;
            }

            // we can have at most 2^32 pieces. this limit is not directly defined but since index
            // in a Peer's Request message is limited to u32 we can infer there must be fewer than
            // 2^32 pieces.
            if info.pieces.len() > 1 << 32 {
                return None;
            }

            info.pieces
                .chunks(20)
                .map(|p| p.try_into().unwrap())
                .collect()
        };

        let trackers = match torrent.announce_list {
            None => vec![vec![torrent.announce.into()]],

            Some(trs) => {
                let mut rng = SmallRng::seed_from_u64(Utc::now().timestamp_millis() as u64);

                // Vec<Vec<&str>> -> Vec<Vec<String>>
                trs.into_iter()
                    .map(|mut tr| {
                        tr.shuffle(&mut rng);
                        tr.into_iter().map(|t| t.into()).collect()
                    })
                    .collect()
            }
        };

        let files = Self::build_files(&info, base_dir)?;
        let total_bytes = files
            .iter()
            .map(|f| f.length)
            .try_fold(0u64, |acc, i| acc.checked_add(i))?;

        Some(Torrent {
            info: Info {
                files,
                piece_length: info.piece_length.try_into().ok()?,
                pieces,
                info_hash: Bencode::hash_dict(buf, "info")?,
                private: info.private == Some(1),
            },
            peers: HashMap::new(),

            trackers,
            next_announce: Utc::now(),

            peer_id,
            bytes_left: total_bytes,
            uploaded: 0,
            downloaded: 0,
        })
    }

    fn build_files(info: &InfoAST, base_dir: &Path) -> Option<Vec<File>> {
        // for single file torrents this is `InfoAST.name`
        // for  multi file torrents this is `InfoAST.name + FileAST.path.join("/")`

        // length and files are mutually exclusive for a valid torrent
        if info.length.is_some() && info.files.is_some() {
            return None;
        }

        let build_file = |length: i64, torrent_dir: &str, path: &[&str]| -> Option<File> {
            if length <= 0 {
                return None;
            }

            // todo: should we check for invalid paths? (incl os-specific blacklists) ?
            let valid_path = |p: &str| p != "." && p != "..";

            let parts = path.iter().filter(|p| valid_path(p)).map(Path::new);
            let file_path =
                PathBuf::from_iter([base_dir, Path::new(torrent_dir)].into_iter().chain(parts));

            // todo: fix this check. should be file_path ~= base_dir + torrent_dir
            // path was empty or all path segments were filtered out
            if file_path == OsStr::new(base_dir) {
                return None;
            }

            Some(File {
                file: file_path,
                length: length.try_into().ok()?,
            })
        };

        // single file case, name is filename
        if let Some(len) = info.length {
            let file = build_file(len, "", &[info.name][..])?;
            return Some(vec![file]);
        }

        // todo: info.name could be "", making all files here top-level
        // todo: validate info.name
        let torrent_dir = info.name;

        info.files // : Option<Vec<_>>
            .as_ref()?
            .iter()
            .map(|file| build_file(file.length, torrent_dir, &file.path))
            .try_collect()
    }

    async fn fetch_peers(&mut self) -> Result<()> {
        if self.next_announce <= Utc::now() && !self.peers.is_empty() {
            return Ok(());
        }

        let client = Client::new();
        let mut tracker_url = String::new();

        // find the first available tracker we can reach, and move it the the front of its own list.
        //
        // for example, if b3 is the first tracker to respond:
        //     [ [a1, a2], [b1, b2, b3], [c1] ]
        //
        // the new tracker list becomes:
        //     [ [a1, a2], [b3, b1, b2], [c1] ]
        //
        // See BEP-12 for more details
        for outer in 0..self.trackers.len() {
            for inner in 0..self.trackers[outer].len() {
                let tracker = &self.trackers[outer][inner];

                // request peers from tracker
                self.build_tracker_url(&mut tracker_url, tracker);
                let Ok((interval, peers)) = Self::get_peers_from_tracker(&client, &tracker_url).await else {
                    tracker_url.clear();
                    continue;
                };

                // make current tracker the first we try next time (in its local inner group, maintaining
                // outer tracker group order)
                self.trackers[outer][..=inner].rotate_right(1);

                // set next tracker update interval
                let interval = Duration::seconds(interval.clamp(0, i64::MAX as u64) as i64);
                self.next_announce = Utc::now() + interval;

                // update our list of peers
                for peer in peers {
                    self.peers.entry(peer).or_insert(None);
                }

                return Ok(());
            }
        }

        Err(Error::NoTrackerAvailable)
    }

    fn build_tracker_url(&self, mut buffer: &mut String, tracker: &str) {
        const UPPERHEX: &[u8; 16] = b"0123456789ABCDEF";

        let mut info_hash = String::with_capacity(60);
        for b in self.info.info_hash {
            info_hash.push('%');
            info_hash.push(UPPERHEX[b as usize >> 4] as char);
            info_hash.push(UPPERHEX[b as usize & 15] as char);
        }

        let _ = write!(
            &mut buffer,
            concat!(
                "{}?",
                "info_hash={info_hash}",
                "&peer_id={peer_id}",
                "&port={port}",
                "&downloaded={downloaded}",
                "&uploaded={uploaded}",
                "&compact={compact}",
                "&left={left}",
            ),
            tracker,
            info_hash = info_hash,
            peer_id = self.peer_id,
            port = 6881,
            downloaded = self.downloaded,
            uploaded = self.uploaded,
            compact = 1,
            left = self.bytes_left, // TODO - need to compute later, not exactly file_length - downloaded
        );
    }

    async fn get_peers_from_tracker(
        client: &Client<HttpConnector>,
        tracker: &str,
    ) -> Result<(u64, Vec<SocketAddrV4>)> {
        let uri = tracker.parse()?;
        let resp = client.get(uri).await?;
        let resp = body::to_bytes(resp).await?;

        let Some(Bencode::Dict(mut tracker)) = Bencode::decode(&resp) else {
            return Err(Error::InvalidTrackerResp {
                reason: None,
            })
        };

        if let Some(fail_msg) = tracker.remove("failure reason") {
            // TODO - avoid allocs
            let reason = fail_msg.str().map(|s| s.into());

            return Err(Error::InvalidTrackerResp { reason });
        }

        // parse response into a (interval, sockaddr's) pair
        let resp = 'resp: {
            try {
                let interval = match tracker.remove("interval")?.num()? {
                    n if n < 0 => break 'resp None,
                    n => n as u64,
                };

                let sock_addrs = match tracker.remove("peers")? {
                    // list of (ip, port) pairs in BE order, format: IIIIPP  (I = Ip, P = Port)
                    Bencode::BStr(peers) if peers.len() % 6 == 0 => {
                        let mut sock_addrs = Vec::with_capacity(peers.len() / 6);

                        for host in peers.chunks(6) {
                            let ipv4 = Ipv4Addr::new(host[0], host[1], host[2], host[3]);
                            let port = BE::read_u16(&host[4..]);

                            sock_addrs.push(SocketAddrV4::new(ipv4, port));
                        }

                        sock_addrs
                    }

                    // list of {"ip", "port"} dicts
                    Bencode::List(peers) => peers
                        .into_iter()
                        .map(|peer| {
                            // todo: the spec defines "peer id" as well, but we do not need it rn and
                            //       not really sure if it exists for all responses
                            let mut peer = peer.dict()?;

                            let ip = peer.remove("ip")?.str()?.parse().ok()?;
                            let port = peer.remove("port")?.str()?.parse().ok()?;

                            Some(SocketAddrV4::new(ip, port))
                        })
                        .try_collect()?,

                    _ => break 'resp None,
                };

                (interval, sock_addrs)
            }
        };

        resp.ok_or(Error::InvalidTrackerResp { reason: None })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::Arc,
    };

    use chrono::Utc;

    use crate::torrent::{File, Info, Torrent};

    #[test]
    fn new() {
        let tor_gen = |base: &Path, prefix: &str| Torrent {
            trackers: vec![
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
                    file: PathBuf::from_iter(
                        [base, Path::new(prefix), Path::new("file.txt")].iter(),
                    ),
                    length: 10,
                }],
                info_hash: if prefix == "" {
                    [
                        11, 5, 171, 161, 242, 160, 178, 230, 220, 146, 241, 219, 17, 67, 62, 95,
                        58, 130, 11, 173,
                    ]
                } else {
                    [
                        116, 83, 104, 101, 231, 122, 204, 114, 242, 152, 196, 136, 195, 44, 49,
                        171, 155, 150, 152, 177,
                    ]
                },
            },
            peer_id: Arc::new("".into()),
            bytes_left: 0,
            uploaded: 0,
            downloaded: 0,
            next_announce: Utc::now(),
            peers: Default::default(),
        };

        let test_files = [
            (&include_bytes!("test_data/mock_dir.torrent")[..], "mock"),
            (&include_bytes!("test_data/mock_file.torrent")[..], ""),
        ];

        for (file, dir_name) in test_files {
            let base_dir = PathBuf::from("/foo");
            let torrent = Torrent::new(file, Arc::new("".into()), &base_dir).unwrap();
            let expected = tor_gen(&base_dir, dir_name);

            assert_eq!(torrent.trackers, expected.trackers);
            assert_eq!(torrent.info, expected.info);
            assert_eq!(torrent.info.info_hash, expected.info.info_hash);
        }
    }

    // #[tokio::test]
    // async fn get_peers() {
    //     let data = include_bytes!("test_data/debian.torrent");
    //     let mut tsunami = Torrent::from_buf(data).unwrap();
    //     let peers = tsunami.fetch_peers().await.unwrap();
    //
    //     println!("{:?}", peers);
    // }
}
