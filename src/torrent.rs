use std::{
    collections::HashMap,
    fmt::Write,
    iter::once,
    net::{Ipv4Addr, SocketAddrV4},
    path::{Path, PathBuf},
    sync::Arc,
};

use byteorder::{ByteOrder, BE};
use hyper::body::Bytes;
use rand::{rngs::SmallRng, seq::SliceRandom, SeedableRng};
use time::{Duration, OffsetDateTime};

use crate::{
    error::{Error, Result},
    peer::Peer,
    torrent_ast::{Bencode, InfoAST, TorrentAST},
    utils::{self, Slice},
};

pub type Sha1Hash = [u8; 20];
pub type Trackers = Slice<String>;

/// Torrent keeps a torrents metadata in a more workable format
#[derive(Debug)]
pub struct Torrent {
    info: Info,
    peers: HashMap<SocketAddrV4, Option<Peer>>,

    // trackers is a group of one or more trackers followed by an optional list of backup groups.
    // this will always contain at least one tracker (`announce_list[0][0]`)
    //
    // example: vec![ vec!["tracker1", "tr2"], vec!["backup1"] ]
    trackers: Slice<Trackers>,
    next_announce: OffsetDateTime,

    peer_id: Arc<String>,
    bytes_left: u64,
    uploaded: u64,
    downloaded: u64,
}

#[derive(Debug, PartialEq)]
struct Info {
    files: Slice<File>,

    piece_length: u32,
    pieces: Slice<Sha1Hash>,
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
    attr: Option<Attr>,
}

#[derive(Debug, PartialEq)]
pub enum Attr {
    Padding,
    Executable,
    Hidden,
    SymLink,
}

impl Torrent {
    pub fn new(buf: &[u8], peer_id: Arc<String>, base_dir: &Path) -> Option<Torrent> {
        Self::validate(&peer_id, base_dir)?;
        let torrent = TorrentAST::decode(buf)?;
        let info = torrent.info;

        let pieces = info
            .pieces
            .chunks(20)
            .map(|p| p.try_into().unwrap())
            .collect();

        let trackers = if let Some(mut trs) = torrent.announce_list {
            let seed = OffsetDateTime::now_utc().unix_timestamp() as u64;
            let mut rng = SmallRng::seed_from_u64(seed);

            trs.iter_mut()
                .map(|tr| {
                    tr.shuffle(&mut rng);
                    tr.into_iter().map(|s| String::from(*s)).collect()
                })
                .collect()
        } else {
            [[torrent.announce.into()].into()].into()
        };

        let files = Self::build_files(&info, base_dir)?;
        let total_bytes = files
            .iter()
            .map(|f| f.length)
            .try_fold(0u64, u64::checked_add)?;

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
            next_announce: OffsetDateTime::now_utc(),

            peer_id,
            bytes_left: total_bytes,
            uploaded: 0,
            downloaded: 0,
        })
    }

    fn validate(peer_id: &str, base_dir: &Path) -> Option<()> {
        if peer_id.len() != 20 {
            return None;
        }

        if !base_dir.has_root() {
            return None;
        }

        Some(())
    }

    fn build_files(info: &InfoAST, base_dir: &Path) -> Option<Slice<File>> {
        // single file case, info.name is filename
        if let Some(len) = info.length {
            let file = File::new(len, base_dir, &[info.name][..])?;
            return Some([file].into());
        }

        let base_dir = {
            let d = utils::valid_path(info.name).then(|| info.name)?;
            base_dir.join(Path::new(d))
        };

        info.files
            .as_ref()?
            .iter()
            .map(|file| File::new(file.length, &base_dir, &file.path))
            .try_collect()
    }

    async fn refresh_peers(&mut self) -> Result<()> {
        if self.next_announce <= OffsetDateTime::now_utc() && !self.peers.is_empty() {
            return Ok(());
        }

        let mut url_buf = String::new();

        // find the first available tracker we can reach and move it the the front of its own list.
        //
        // for example, if b3 is the first tracker to respond:
        //     [ [a1, a2], [b1, b2, b3], [c1] ]
        //
        // the new tracker list becomes:
        //     [ [a1, a2], [b3, b1, b2], [c1] ]
        //
        // See BEP-12 for more details
        for group in 0..self.trackers.len() {
            for index in 0..self.trackers[group].len() {
                let tracker = &self.trackers[group][index];
                self.build_tracker_url(tracker, &mut url_buf);

                // request peers from tracker
                let body = utils::get_body(&url_buf).await?;
                let Ok((interval, peers)) = Self::parse_tracker_resp(body) else {
                    continue;
                };

                // make current tracker the first we try next time (in its local inner group, maintaining
                // outer tracker group order)
                self.trackers[group][..=index].rotate_right(1);

                // set next tracker update interval, min 5m
                let interval = Duration::seconds(interval.clamp(300, i64::MAX as u64) as i64);
                self.next_announce = OffsetDateTime::now_utc() + interval;

                // update our list of peers
                for peer in peers {
                    self.peers.entry(peer).or_insert(None);
                }

                return Ok(());
            }
        }

        Err(Error::NoTrackerAvailable)
    }

    fn build_tracker_url(&self, tracker: &str, mut buffer: &mut String) {
        const HEXES: &[u8; 16] = b"0123456789ABCDEF";
        buffer.clear();

        let mut info_hash = String::with_capacity(60);
        for b in self.info.info_hash {
            info_hash.push('%');
            info_hash.push(HEXES[b as usize >> 4] as char);
            info_hash.push(HEXES[b as usize & 15] as char);
        }

        let _ = write!(
            &mut buffer,
            "{tracker}?info_hash={}&peer_id={}&port={}&downloaded={}&uploaded={}&compact={}&left={}",
            info_hash,
            self.peer_id,
            6881,
            self.downloaded,
            self.uploaded,
            1,
            self.bytes_left,
        );
    }

    fn parse_tracker_resp(resp: Bytes) -> Result<(u64, Vec<SocketAddrV4>)> {
        // todo: propagate error
        let Some(mut tracker) = (try { Bencode::decode(&resp)?.dict()? }) else {
            return Err(Error::InvalidTrackerResp(None))
        };

        // TODO - avoid allocs
        if let Some(fail_msg) = tracker.remove(&b"failure reason"[..]) {
            let reason = try { fail_msg.str()?.into() };
            return Err(Error::InvalidTrackerResp(reason));
        }

        // parse response into a (interval, sockaddr's) pair
        let parse_resp = try {
            let interval = tracker.remove(&b"interval"[..])?.num()?.try_into().ok()?;
            let peers = tracker.remove(&b"peers"[..])?;

            let sock_addrs = if let Bencode::Str(peers) = peers {
                peers
                    .chunks(6)
                    .map(|host| {
                        let ipv4 = Ipv4Addr::new(host[0], host[1], host[2], host[3]);
                        let port = BE::read_u16(&host[4..]);

                        SocketAddrV4::new(ipv4, port)
                    })
                    .collect()
            } else if let Bencode::List(peers) = peers {
                peers
                    .into_iter()
                    .map(|peer| {
                        let mut peer = peer.dict()?;
                        let ip = peer.remove(&b"ip"[..])?.str()?.parse().ok()?;
                        let port = peer.remove(&b"port"[..])?.str()?.parse().ok()?;

                        Some(SocketAddrV4::new(ip, port))
                    })
                    .try_collect()?
            } else {
                return Err(Error::InvalidTrackerResp(None));
            };

            (interval, sock_addrs)
        }: Option<_>;

        parse_resp.ok_or(Error::InvalidTrackerResp(None))
    }
}

impl File {
    fn new(length: i64, torrent_dir: &Path, paths: &[&str]) -> Option<File> {
        if length <= 0 {
            return None;
        }

        // todo: os specific clean_path fns
        let parts = paths.iter().filter(|p| utils::valid_path(p)).map(Path::new);
        let file_path = PathBuf::from_iter(once(torrent_dir).into_iter().chain(parts));

        // parts were empty or all path segments were filtered out
        if file_path.ends_with(torrent_dir) {
            return None;
        }

        Some(File {
            file: file_path,
            length: length.try_into().ok()?,
            attr: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::Arc,
    };

    use time::OffsetDateTime;

    use crate::torrent::{File, Info, Torrent};

    #[test]
    fn new() {
        let tor_gen = |base: &Path, prefix: &str| Torrent {
            trackers: [
                ["http://tracker.example.com".into()].into(),
                ["http://tracker2.example.com".into()].into(),
            ]
            .into(),
            info: Info {
                piece_length: 32768,
                pieces: [[
                    0, 72, 105, 249, 236, 50, 141, 28, 177, 230, 77, 80, 106, 67, 249, 35, 207,
                    173, 235, 151,
                ]]
                .into(),
                private: true,
                files: [File {
                    file: PathBuf::from_iter(
                        [base, Path::new(prefix), Path::new("file.txt")].iter(),
                    ),
                    length: 10,
                    attr: None,
                }]
                .into(),
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
            next_announce: OffsetDateTime::now_utc(),
            peers: Default::default(),
        };

        let test_files = [
            (&include_bytes!("test_data/mock_dir.torrent")[..], "mock"),
            (&include_bytes!("test_data/mock_file.torrent")[..], ""),
        ];

        let peer_id: Arc<String> = Arc::new("-TS0001-|testClient|".into());

        for (file, dir_name) in test_files {
            let base_dir = PathBuf::from("/foo");
            let torrent = Torrent::new(file, peer_id.clone(), &base_dir).unwrap();
            let expected = tor_gen(&base_dir, dir_name);

            assert_eq!(torrent.trackers, expected.trackers);
            assert_eq!(torrent.info, expected.info);
            assert_eq!(torrent.info.info_hash, expected.info.info_hash);
        }
    }

    // #[tokio::test]
    // async fn get_peers() {
    //     let data = include_bytes!("test_data/debian.torrent");
    //     let base_dir = env::temp_dir();
    //
    //     let mut tsunami = Tsunami::new(base_dir).unwrap();
    //     let torrent = tsunami.add_torrent(data).unwrap();
    //     torrent.refresh_peers().await.unwrap();
    //     println!("{:?}", torrent.peers.keys());
    // }
}
