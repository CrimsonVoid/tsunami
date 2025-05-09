use std::{path::PathBuf, sync::Arc};

use rand::{Rng, SeedableRng, distributions::Alphanumeric, rngs::SmallRng};
use time::OffsetDateTime;

use crate::torrent::Torrent;

/// Tsunami bittorrent client
pub(crate) struct Tsunami {
    peer_id: Arc<String>,
    base_dir: PathBuf,
    torrents: Vec<Torrent>,
}

impl Tsunami {
    pub fn new(base_dir: PathBuf) -> Option<Tsunami> {
        // todo: peer_id should be identifiable for user/clients/machine
        let seed = OffsetDateTime::now_utc().unix_timestamp() as u64;
        let rng = SmallRng::seed_from_u64(seed);
        let peer_id = Arc::new(format!(
            "-TS0001-{}",
            rng.sample_iter(&Alphanumeric)
                .take(12)
                .map(char::from)
                .collect::<String>()
        ));

        if !base_dir.has_root() {
            return None;
        }

        Some(Tsunami {
            peer_id,
            base_dir,
            torrents: vec![],
        })
    }

    pub fn add_torrent(&mut self, buf: &[u8]) -> Option<&mut Torrent> {
        let torrent = Torrent::new(buf, self.peer_id.clone(), &self.base_dir)?;
        self.torrents.push(torrent);
        self.torrents.last_mut()
    }
}
