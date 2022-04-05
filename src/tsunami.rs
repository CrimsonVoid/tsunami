use std::{env::temp_dir, path::PathBuf, sync::Arc};

use chrono::Utc;
use rand::{distributions::Alphanumeric, rngs::SmallRng, Rng, SeedableRng};

use crate::torrent::Torrent;

/// Tsunami bittorrent client
pub struct Tsunami {
    peer_id: Arc<String>,
    base_dir: PathBuf,
    torrents: Vec<Torrent>,
}

impl Tsunami {
    pub fn new() -> Option<Tsunami> {
        let rng = SmallRng::seed_from_u64(Utc::now().timestamp_millis() as u64);
        // todo: peer_id should be identifiable for user/clients/machine
        let peer_id = Arc::new(format!(
            "-TS0001-{}",
            rng.sample_iter(&Alphanumeric)
                .take(12)
                .map(char::from)
                .collect(): String
        ));

        Some(Tsunami {
            peer_id,
            base_dir: download_dir(),
            torrents: vec![],
        })
    }

    // pub fn add_torrent(&mut self, buf: &[u8]) {
    //     let torrent = Torrent::from_buf(buf, self.peer_id.clone(), &self.base_dir)?;
    //     self.torrents.push(torrent);
    // }
}

fn download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(temp_dir)
}

#[cfg(test)]
mod tests {}
