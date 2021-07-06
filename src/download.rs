use crate::torrent::Torrent;
use futures::stream::{Stream, StreamExt};
use hyper::{body::HttpBody, Client};
use std::error::Error;

struct Tsunami {
    torrent: Torrent,

    peer_id: String,
    file_length: u64,
}

impl Tsunami {
    pub fn new(torrent_file: &str) -> Option<Tsunami> {
        let torrent = Torrent::decode(torrent_file)?;
        let file_length = torrent.info.files.iter().map(|f| f.length).sum();

        Some(Tsunami {
            torrent,
            peer_id: "-TS0001-hellotsunami"[..20].into(),
            file_length,
        })
    }

    async fn tracker_handshake(&self) -> Result<String, Box<dyn Error>> {
        let uri = self.tracker().parse()?;
        let client = Client::new();
        let mut resp = client.get(uri).await?;

        // TODO - streaming body parser
        // TODO - return list of peers

        let mut body = Vec::new();
        while let Some(chunk) = resp.body_mut().data().await {
            let chunk = chunk?;

            println!("extending: {:?}", &chunk);
            body.extend(&chunk);
        }

        // let body = hyper::body::to_bytes(resp.into_body()).await?;
        match std::str::from_utf8(&body) {
            Ok(s) => Ok(s.into()),
            Err(e) => {
                let bs = &body[..e.valid_up_to()];
                Ok(std::str::from_utf8(&bs).map(|s| s.into())?)
            }
        }
    }

    fn tracker(&self) -> String {
        // we must have at least one tracker
        let tracker = &self.torrent.announce_list[0][0];

        const _UPPERHEX: &[u8; 16] = b"0123456789ABCDEF";

        let mut info_hash = String::with_capacity(60);
        for b in self.torrent.info_hash {
            info_hash.push('%');
            info_hash.push(_UPPERHEX[b as usize >> 4] as char);
            info_hash.push(_UPPERHEX[b as usize & 15] as char);
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
            downloaded = 0,
            uploaded = 0,
            compact = 1,
            left = self.file_length,
        )
    }

    // fn parse_tracker_resp(resp: &)
}

#[cfg(test)]
mod tests {
    use super::Tsunami;
    use std::str::from_utf8_unchecked;

    #[tokio::test]
    async fn decode_torrent() {
        let data = unsafe { from_utf8_unchecked(include_bytes!("test_data/debian.torrent")) };
        let tsunami = Tsunami::new(data).unwrap();

        let resp = tsunami.tracker_handshake().await;

        println!("{:?}", resp);
    }
}
