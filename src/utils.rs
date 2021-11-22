use crate::error::Result;

use async_trait::async_trait;
use tokio::net::TcpStream;

pub trait IterExt: Iterator {
    fn flat_map_all<T, U>(self, op: impl Fn(T) -> Option<U>) -> Option<Vec<U>>
    where
        Self: Iterator<Item = T> + Sized,
    {
        let (min, max) = self.size_hint();
        let mut vs = Vec::with_capacity(max.unwrap_or(min));

        for b in self {
            vs.push(op(b)?);
        }

        Some(vs)
    }
}

impl<I: Iterator + Sized> IterExt for I {}

#[async_trait]
pub(crate) trait TcpStreamExt {
    async fn handshake(&self) -> Result<()>;
}

#[async_trait]
impl TcpStreamExt for TcpStream {
    async fn handshake(&self) -> Result<()> {
        unimplemented!()
    }
}

// #[async_trait]
// pub trait ClientExt {
//     async fn get_peers(&self, url: &str) -> Result<(u64, Vec<SocketAddrV4>)>;
// }

// #[async_trait]
// impl ClientExt for Client<HttpConnector> {
//     async fn get_peers(&self, tracker: &str) -> Result<(u64, Vec<SocketAddrV4>)> {
//         let uri = tracker.parse()?;
//         let resp = self.get(uri).await?;
//         let resp = body::to_bytes(resp).await?;

//         let Some(Bencode::Dict(mut tracker)) = Bencode::decode(&resp) else {
//                 return Err(Error::InvalidTrackerResp {
//                     failure_reason: None,
//                 });
//         };

//         if let Some(fail_msg) = tracker.remove("failure reason") {
//             // TODO - avoid allocs
//             let failure_reason = fail_msg.str().map(|s| s.into());

//             return Err(Error::InvalidTrackerResp { failure_reason });
//         }

//         // parse response into a (interval, sockaddr's) pair
//         // use a function here to simplify control flow since most parsing operations return
//         // an Option
//         let resp = 'resp: {
//             try {
//                 let interval = match tracker.remove("interval")?.num()? {
//                     n if n < 0 => break 'resp None,
//                     n => n as u64,
//                 };

//                 // let sock_addrs = match tracker.remove("peers")? {
//                 let sock_addrs = match tracker.remove("peers")? {
//                     // peers is a list of IpPort pairs in big-ending order. the first four bytes
//                     // represent the ip and the last two the port
//                     // binary format: IIIIPP  (I = Ip, P = Port)
//                     Bencode::BStr(peers) if peers.len() % 6 == 0 => {
//                         let mut sock_addrs = Vec::with_capacity(peers.len() / 6);

//                         for host in peers.chunks(6) {
//                             let ipv4 = Ipv4Addr::new(host[0], host[1], host[2], host[3]);
//                             let port = be_u16::<_, ()>(&host[4..]).ok()?.1;

//                             sock_addrs.push(SocketAddrV4::new(ipv4, port));
//                         }

//                         sock_addrs
//                     }

//                     // peers is a list of dicts each containing an "ip" and "port" key
//                     // the spec defines "peer id" as well, but we do not need it rn and not really sure
//                     // if it exists for all responses
//                     Bencode::List(peers) => peers.into_iter().flat_map_all(|peer| {
//                         let mut peer = peer.dict()?;

//                         let ip = peer.remove("ip")?.str()?.parse().ok()?;
//                         let port = peer.remove("port")?.str()?.parse().ok()?;

//                         Some(SocketAddrV4::new(ip, port))
//                     })?,

//                     _ => break 'resp None,
//                 };

//                 (interval, sock_addrs)
//             }
//         };

//         resp.ok_or(Error::InvalidTrackerResp {
//             failure_reason: None,
//         })
//     }
// }
