use std::{
    collections::HashMap,
    fmt::Write,
    iter::once,
    net::{Ipv4Addr, SocketAddrV4},
    path::{Path, PathBuf},
    sync::Arc,
};

use bytes::Bytes;
use rand::{SeedableRng, rngs::SmallRng, seq::SliceRandom};
use reqwest::Client;
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
pub(crate) struct Torrent {
    pub info: Info,
    pub peers: HashMap<SocketAddrV4, Option<Peer>>,

    // trackers is a group of one or more trackers followed by an optional list of backup groups.
    // this will always contain at least one tracker (`announce_list[0][0]`)
    //
    // example: vec![ vec!["tracker1", "tr2"], vec!["backup1"] ]
    pub trackers: Slice<Trackers>,
    pub next_announce: OffsetDateTime,

    pub peer_id: Arc<String>,
    pub bytes_left: u64,
    pub uploaded: u64,
    pub downloaded: u64,
}

#[derive(Debug, PartialEq)]
pub(crate) struct Info {
    pub files: Slice<File>,

    pub piece_length: u32,
    pub pieces: Slice<Sha1Hash>,
    pub info_hash: Sha1Hash,

    pub private: bool,
}

#[derive(Debug, PartialEq)]
pub(crate) struct File {
    // absolute location where file is saved. this defaults to base_path, but may be sanitized for
    // OS-specific character limitations or other malformed file names
    // default: OS_DOWNLOAD_DIR | HOME + base_path
    pub file: PathBuf,
    pub length: u64,
    pub attr: Option<Attr>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Attr {
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

        let trackers = if let Some(mut trs) = torrent.announceList {
            let seed = OffsetDateTime::now_utc().unix_timestamp() as u64;
            let mut rng = SmallRng::seed_from_u64(seed);

            trs.iter_mut()
                .map(|tr| {
                    tr.shuffle(&mut rng);
                    tr.iter_mut().map(|s| String::from(*s)).collect()
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
                piece_length: info.pieceLength.try_into().ok()?,
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
            let d = utils::valid_path(info.name).then_some(info.name)?;
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
        let client = Client::new();

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
                let body = utils::get_body(&client, &url_buf).await?;
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

    fn build_tracker_url(&self, tracker: &str, buffer: &mut String) {
        const HEXES: &[u8; 16] = b"0123456789ABCDEF";
        buffer.clear();

        let mut info_hash = String::with_capacity(60);
        for b in self.info.info_hash {
            info_hash.extend([
                '%',
                HEXES[b as usize >> 4] as char,
                HEXES[b as usize & 15] as char,
            ]);
        }

        let _ = write!(
            buffer,
            "{tracker}?info_hash={}&peer_id={}&port={}&downloaded={}&uploaded={}&compact={}&left={}",
            info_hash, self.peer_id, 6881, self.downloaded, self.uploaded, 1, self.bytes_left,
        );
    }

    fn parse_tracker_resp(resp: Bytes) -> Result<(u64, Vec<SocketAddrV4>)> {
        // todo: propagate error
        let Some(mut tracker) = (try { Bencode::decode(&resp)?.dict()? }) else {
            return Err(Error::InvalidTrackerResp(None));
        };

        // TODO - avoid allocs
        if let Some(fail_msg) = tracker.remove(&b"failure reason"[..]) {
            let reason = try { fail_msg.str()?.into() };
            return Err(Error::InvalidTrackerResp(reason));
        }

        // parse response into a (interval, sockaddr's) pair
        let parse_resp: Option<_> = try {
            let interval = tracker.remove(&b"interval"[..])?.num()?.try_into().ok()?;
            let peers = tracker.remove(&b"peers"[..])?;

            let sock_addrs = if let Bencode::Str(peers) = peers {
                let mut addrs = Vec::with_capacity(peers.len() / 6);

                for host in peers.chunks(6) {
                    let ipv4 = Ipv4Addr::new(host[0], host[1], host[2], host[3]);
                    let port = u16::from_be_bytes(host[4..].try_into().unwrap());

                    addrs.push(SocketAddrV4::new(ipv4, port));
                }

                addrs
            } else if let Bencode::List(peers) = peers {
                let mut addrs = Vec::with_capacity(peers.len());

                for peer in peers {
                    let mut peer = peer.dict()?;
                    let ip = peer.remove(&b"ip"[..])?.str()?.parse().ok()?;
                    let port = peer.remove(&b"port"[..])?.str()?.parse().ok()?;

                    addrs.push(SocketAddrV4::new(ip, port));
                }

                addrs
            } else {
                return Err(Error::InvalidTrackerResp(None));
            };

            (interval, sock_addrs)
        };

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
        let file_path = PathBuf::from_iter(once(torrent_dir).chain(parts));

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
