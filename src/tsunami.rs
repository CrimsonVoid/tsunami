use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
    net::{Ipv4Addr, SocketAddrV4},
};

use chrono::{DateTime, Duration, Utc};
use hyper::{body, client::HttpConnector, Client};
use nom::number::complete::be_u16;
use rand::{distributions::Alphanumeric, prelude::SliceRandom, thread_rng, Rng};

use crate::{
    bencode::Bencode,
    error::{Error, Result},
    torrent::Torrent,
    utils::IterExt,
};

/// Tsunami bittorrent client
pub struct Tsunami {
    torrent: Torrent,

    peer_id: String,
    file_length: u64,

    uploaded: u64,
    downloaded: u64,

    tracker_interval: DateTime<Utc>,
    peers: HashSet<SocketAddrV4>,
}

impl Tsunami {
    pub fn new(torrent_file: &[u8]) -> Option<Tsunami> {
        let mut torrent = Torrent::decode(torrent_file)?;

        let file_length = torrent.info.files.iter().map(|f| f.length).sum();

        // shuffle each group of trackers
        let mut rng = rand::thread_rng();
        torrent
            .trackers_list
            .iter_mut()
            .for_each(|tl| tl.shuffle(&mut rng));

        let peer_id = format!(
            "-TS0001-{}",
            thread_rng()
                .sample_iter(&Alphanumeric)
                .take(12)
                .map(char::from)
                .collect::<String>()
        );

        Some(Tsunami {
            torrent,
            peer_id,
            file_length,
            uploaded: 0,
            downloaded: 0,
            tracker_interval: Utc::now(),
            peers: HashSet::new(),
        })
    }

    pub async fn get_peers(&mut self) -> Result<&HashSet<SocketAddrV4>> {
        if self.tracker_interval <= Utc::now() {
            return Ok(&self.peers);
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
        for outer in 0..self.torrent.trackers_list.len() {
            for inner in 0..self.torrent.trackers_list[outer].len() {
                let tracker = &self.torrent.trackers_list[outer][inner];

                self.build_tracker_url(&mut tracker_url, &tracker);
                let resp = Self::get_peers_from_tracker(&client, &tracker_url);

                if let Ok((interval, peers)) = resp.await {
                    self.torrent.trackers_list[outer][..=inner].rotate_right(1);

                    let interval = Duration::seconds(interval.clamp(0, i64::MAX as u64) as i64);
                    self.tracker_interval = Utc::now() + interval;
                    self.peers.extend(peers.into_iter());

                    return Ok(&self.peers);
                }
                tracker_url.clear();
            }
        }

        Err(Error::NoTrackerAvailable)
    }

    fn build_tracker_url(&self, mut buffer: &mut String, tracker: &str) {
        const UPPERHEX: &[u8; 16] = b"0123456789ABCDEF";

        let mut info_hash = String::with_capacity(60);
        for b in self.torrent.info_hash {
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
            left = self.file_length, // TODO - need to compute later, not exactly file_length - downloaded
        );
    }

    pub async fn get_peers_from_tracker(
        client: &Client<HttpConnector>,
        tracker: &str,
    ) -> Result<(u64, Vec<SocketAddrV4>)> {
        let uri = tracker.parse()?;
        let resp = client.get(uri).await?;
        let resp = body::to_bytes(resp).await?;

        let mut tracker = match Bencode::decode(&resp) {
            Some(Bencode::Dict(d)) => d,
            _ => {
                return Err(Error::InvalidTrackerResp {
                    failure_reason: None,
                })
            }
        };

        if let Some(fail_msg) = tracker.remove("failure reason") {
            // TODO - avoid allocs
            let failure_reason = fail_msg.str().map(|s| s.into());

            return Err(Error::InvalidTrackerResp { failure_reason });
        }

        // parse response into a (interval, sockaddr's) pair
        // use a function here to simplify control flow since most parsing operations return
        // an Option
        let resp = |mut tracker: HashMap<&str, Bencode>| -> Option<_> {
            let interval = match tracker.remove("interval")?.num()? {
                n if n < 0 => return None,
                n => n as u64,
            };

            let sock_addrs = match tracker.remove("peers")? {
                // peers is a list of IpPort pairs in big-ending order. the first four bytes
                // represent the ip and the last two the port
                // binary format: IIIIPP  (I = Ip, P = Port)
                Bencode::BStr(peers) if peers.len() % 6 == 0 => {
                    let mut sock_addrs = Vec::with_capacity(peers.len() / 6);

                    for host in peers.chunks(6) {
                        let ipv4 = Ipv4Addr::new(host[0], host[1], host[2], host[3]);
                        let port = be_u16::<_, ()>(&host[4..]).ok()?.1;

                        sock_addrs.push(SocketAddrV4::new(ipv4, port));
                    }

                    sock_addrs
                }

                // peers is a list of dicts each containing an "ip" and "port" key
                // the spec defines "peer id" as well, but we do not need it rn and not really sure
                // if it exists for all responses
                Bencode::List(peers) => peers.into_iter().flat_map_all(|peer| {
                    let mut peer = peer.dict()?;

                    let ip = peer.remove("ip")?.str()?.parse().ok()?;
                    let port = peer.remove("port")?.str()?.parse().ok()?;

                    Some(SocketAddrV4::new(ip, port))
                })?,

                _ => return None,
            };

            Some((interval, sock_addrs))
        }(tracker);

        resp.ok_or(Error::InvalidTrackerResp {
            failure_reason: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Tsunami;

    #[tokio::test]
    async fn get_peers() {
        let data = include_bytes!("test_data/debian.torrent");
        let mut tsunami = Tsunami::new(data).unwrap();
        let peers = tsunami.get_peers().await.unwrap();

        println!("{:?}", peers);
    }
}
