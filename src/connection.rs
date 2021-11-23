use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub(crate) struct Connection {
    choked: bool,
    interested: bool,

    conn: TcpStream,
    peer_id: String,
}

impl Connection {
    pub(crate) async fn handshake(
        mut conn: TcpStream,
        info_hash: &[u8],
        peer_id: &[u8],
    ) -> Option<Connection> {
        // Handshake layout:
        // length | value
        // -------+-------------------
        //      1 | 19 (\x13)  =>  1
        //     19 | Bittorrent Protocol
        //      8 | extn flags \x00 * 8
        //     20 | sha-1
        //     20 | peer_id
        //     -- | total
        //     68
        let (mut rx, mut tx) = conn.split();

        // write our end of the handshake
        let send = async {
            let prefix = b"\x13Bittorrent Protocol\x00\x00\x00\x00\x00\x00\x00\x00";
            tx.write_all(prefix).await?;
            tx.write_all(info_hash).await?;
            tx.write_all(peer_id).await?;
            Ok(())
        };

        // read a bittorrent greeting
        let recv = async {
            let err = Err(std::io::Error::from(std::io::ErrorKind::Other));
            let mut buffer = vec![0; 20];

            // protocol prefix
            rx.read_exact(&mut buffer).await?;
            if &buffer[..19] != b"\x13Bittorrent Protocol" {
                return err;
            }

            // extension flags
            rx.read_exact(&mut buffer[..8]).await?;
            if !&buffer[..8].iter().all(|b| *b == 0) {
                return err;
            }

            // info_hash
            rx.read_exact(&mut buffer).await?;
            if buffer != info_hash {
                return err;
            }

            // peer id
            buffer.fill(0);
            rx.read_exact(&mut buffer[..]).await?;
            String::from_utf8(buffer).or(err)
        };

        let (_, peer_id) = futures::try_join!(send, recv).ok()?;

        Some(Connection {
            choked: false,
            interested: false,
            conn,
            peer_id,
        })
    }
}
