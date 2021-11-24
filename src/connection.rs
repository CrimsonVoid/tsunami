use crate::error::Result;
use bitvec::prelude::{BitArray, BitVec, Lsb0};
use bitvec::{bitarr, bitvec};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub(crate) struct Connection {
    // status is a bitfield
    // 0b000x = self choked
    // 0b00x0 = self interested
    // 0b0x00 = peer choked
    // 0bx000 = peer interested
    status: BitArray<Lsb0, [u8; 1]>,
    bitfield: BitVec,
    conn: TcpStream,
}

impl Connection {
    const MAX_PIECE_LENGTH: u32 = 1 << 14; // 16 KiB

    // bitfield markers for `[Connection::status]`
    const SELF_CHOKED: u8 = 1 << 0;
    const SELF_INTERESTED: u8 = 1 << 1;
    const PEER_CHOKED: u8 = 1 << 2;
    const PEER_INTERESTED: u8 = 1 << 3;

    pub(crate) async fn handshake(
        mut conn: TcpStream,
        info_hash: &[u8],
        peer_id: &[u8],
        total_pieces: usize,
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

        let b: BitArray<Lsb0, [u8; 1]> = bitarr![Lsb0, u8; 0, 1, 0, 1];

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

        let t  = bitvec![0; total_pieces];
        let t2: &[u8] = t.as_raw_slice().

        Some(Connection {
            status: bitarr![const Lsb0, u8; 0, 1, 0, 1],
            bitfield: bitvec![0; total_pieces],
            conn,
        })
    }

    pub(crate) async fn decode_frame(&mut self) -> Result<Message> {
        // message format: <length: u32> <message type: u8> <payload?: Vec<u8>>
        let length = self.conn.read_u32().await.unwrap();

        if length == 0 {
            return Ok(Message::KeepAlive);
        }

        Ok(match self.conn.read_u8().await.unwrap() {
            0 if length == 1 => Message::Choke,
            1 if length == 1 => Message::Unchoke,
            2 if length == 1 => Message::Interested,
            3 if length == 1 => Message::NotInterested,
            4 if length == 5 => Message::Have(self.conn.read_u32().await.unwrap()),
            5 if length == (1 + self.bitfield.len()) as u32 => Message::Bitfield(vec![]), // todo - verify bitfield length
            6 if length == 13 => Message::Request {
                index: self.conn.read_u32().await.unwrap(),
                begin: self.conn.read_u32().await.unwrap(),
                length: self.conn.read_u32().await.unwrap(),
            },
            7 if length >= 9 && length - 9 < Self::MAX_PIECE_LENGTH => Message::Piece {
                index: self.conn.read_u32().await.unwrap(),
                begin: self.conn.read_u32().await.unwrap(),
                block: vec![],
            },
            8 if length == 13 => Message::Cancel {
                index: self.conn.read_u32().await.unwrap(),
                begin: self.conn.read_u32().await.unwrap(),
                length: self.conn.read_u32().await.unwrap(),
            },
            9 if length == 3 => Message::Port(self.conn.read_u16().await.unwrap()),
            _ => return Err(crate::error::Error::NoTrackerAvailable), // todo - remove
        })
    }
}

pub(crate) enum Message {
    KeepAlive,                        //        | len = 0
    Choke,                            // id = 0 | len = 1
    Unchoke,                          // id = 1 | len = 1
    Interested,                       // id = 2 | len = 1
    NotInterested,                    // id = 3 | len = 1
    Have(/* piece index */ u32),      // id = 4 | len = 5
    Bitfield(/* bitfield */ Vec<u8>), // id = 5 | len = 1+x
    // id = 6 | len = 13
    Request {
        index: u32,
        begin: u32,
        length: u32,
    },
    // id = 7 | len = 9+x
    Piece {
        index: u32,
        begin: u32,
        block: Vec<u8>,
    },
    // id = 8 | len = 13
    Cancel {
        index: u32,
        begin: u32,
        length: u32,
    },
    Port(/* listen port */ u16), // id = 9 | len = 3
}

#[cfg(test)]
mod test {
    use bitvec::prelude::*;
    use std::mem::{size_of, size_of_val};

    #[test]
    fn arr_size() {
        let b: BitArray<Lsb0, [u32; 2]> = bitarr![Lsb0, u32; 0; 55];
        let b2: BitArray = bitarr![0; 4];
        // let b2: usize = 0;

        println!("{}", size_of_val(&b));
        println!("{}", size_of_val(&b2));
        println!("{}", size_of::<usize>());
    }
}
