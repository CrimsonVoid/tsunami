use crate::bencode::Bencode;
use crate::error::{TError, TResult};
use crate::torrent::Torrent;
use crate::utils::IterExt;
use hyper::client::HttpConnector;
use hyper::{body, Client};
use nom::number::complete::be_u16;
use rand::prelude::SliceRandom;
use std::cell::Cell;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};

pub struct Tsunami {
    torrent: Torrent,

    peer_id: String,
    file_length: u64,

    uploaded: u64,
    downloaded: u64,

    // offsets to use when trying to get the next tracker from torrent.annouce_list
    trackers_offset: Cell<(usize, usize)>,
    // TODO - track interval
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

        Some(Tsunami {
            torrent,
            peer_id: "-TS0001-hellotsunami"[..20].into(), // TODO - randomize
            file_length,
            uploaded: 0,
            downloaded: 0,
            trackers_offset: Cell::new((0, 0)),
        })
    }

    // pub async fn get_peers(&self) -> TResult<&[SocketAddrV4]> {
    //     // should check for interval before requesting new peers from trackers
    //     // return existing list of peers

    //     todo!()
    // }

    pub async fn tracker_handshake(&self) -> TResult<(u64, Vec<SocketAddrV4>)> {
        let client = Client::new();

        // track our starting offsets so we know when we've looped around
        let (start_outer, mut start_inner) = self.trackers_offset.get();
        // track current position
        let (mut outer, mut inner) = (start_outer, start_inner);

        let trackers = &self.torrent.trackers_list[..];

        while {
            while {
                match self.get_tracker_resp(&client).await {
                    Some(r) => return Ok(r),
                    _ => {}
                }

                // increment inner by 1, looping back around if we reach past the end and
                // update trackers_offset
                inner += 1;
                if inner >= trackers[outer].len() {
                    inner = 0;
                }
                self.trackers_offset.replace((outer, inner));

                inner != start_inner
            } {}

            // reset inner
            start_inner = 0;
            inner = 0;

            // increment outer by 1, looping back around if we reach past the end and
            // update trackers_offset
            outer += 1;
            if outer >= trackers.len() {
                outer = 0;
            }
            self.trackers_offset.replace((outer, inner));

            outer != start_outer
        } {}

        // we exhausted all available trackers, reset our position for the next time we try
        self.trackers_offset.replace((0, 0));

        Err(TError::NoTrackerAvailable)
    }

    async fn get_tracker_resp(
        &self,
        client: &Client<HttpConnector>,
    ) -> Option<(u64, Vec<SocketAddrV4>)> {
        // TODO - don't discard errors

        let uri = self.tracker_url().parse().ok()?;

        let resp = client.get(uri).await.ok()?;
        let body = body::to_bytes(resp).await.ok()?;

        Self::parse_tracker_resp(&body).ok()
    }

    fn tracker_url(&self) -> String {
        // we must have at least one tracker
        // TODO - shuffle and rotate trackers
        let (i, j) = self.trackers_offset.get();
        let tracker = &self.torrent.trackers_list[i][j];

        const UPPERHEX: &[u8; 16] = b"0123456789ABCDEF";

        let mut info_hash = String::with_capacity(60);
        for b in self.torrent.info_hash {
            info_hash.push('%');
            info_hash.push(UPPERHEX[b as usize >> 4] as char);
            info_hash.push(UPPERHEX[b as usize & 15] as char);
        }

        format!(
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
        )
    }

    fn parse_tracker_resp(resp: &[u8]) -> TResult<(u64, Vec<SocketAddrV4>)> {
        let mut tracker = match Bencode::decode(resp) {
            Some(Bencode::Dict(d)) => d,
            _ => {
                return Err(TError::InvalidTrackerResp {
                    failure_reason: None,
                })
            }
        };

        if let Some(fail_msg) = tracker.remove("failure reason") {
            let failure_reason = fail_msg.str().map(|s| s.into());

            return Err(TError::InvalidTrackerResp { failure_reason });
        }

        // use a function here to simplify control flow since most parsing operations return
        // an Option
        let t = |mut tracker: HashMap<&str, Bencode>| -> Option<_> {
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
                // it exists for all responses
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

        match t {
            Some(t) => Ok(t),
            None => Err(TError::InvalidTrackerResp {
                failure_reason: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Tsunami;

    #[tokio::test]
    async fn decode_torrent() {
        let data = include_bytes!("test_data/debian.torrent");
        let tsunami = Tsunami::new(data).unwrap();
        let resp = tsunami.tracker_handshake().await.unwrap();

        println!("{:?}", resp);
    }
}
