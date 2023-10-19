use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use time::OffsetDateTime;

use crate::{
    tests::test_data,
    torrent::{File, Info, Torrent},
};

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
                0, 72, 105, 249, 236, 50, 141, 28, 177, 230, 77, 80, 106, 67, 249, 35, 207, 173, 235,
                151,
            ]]
            .into(),
            private: true,
            files: [File {
                file: PathBuf::from_iter([base, Path::new(prefix), Path::new("file.txt")].iter()),
                length: 10,
                attr: None,
            }]
            .into(),
            info_hash: if prefix == "" {
                [
                    11, 5, 171, 161, 242, 160, 178, 230, 220, 146, 241, 219, 17, 67, 62, 95, 58, 130,
                    11, 173,
                ]
            } else {
                [
                    116, 83, 104, 101, 231, 122, 204, 114, 242, 152, 196, 136, 195, 44, 49, 171, 155,
                    150, 152, 177,
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
        //
        (test_data::MOCK_DIR, "mock"),
        (test_data::MOCK_FILE, ""),
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
//     let data = test_data::DEBIAN_FILE;
//     let base_dir = env::temp_dir();
//
//     let mut tsunami = Tsunami::new(base_dir).unwrap();
//     let torrent = tsunami.add_torrent(data).unwrap();
//     torrent.refresh_peers().await.unwrap();
//     println!("{:?}", torrent.peers.keys());
// }
