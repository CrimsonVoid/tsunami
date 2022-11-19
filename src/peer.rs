use std::{io, io::IoSlice};

use bitvec::prelude::{bitbox, BitBox, Lsb0};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufStream},
    net::{TcpStream, ToSocketAddrs},
};

use crate::error::{DecodeError, Result};

#[derive(Debug)]
pub(crate) struct Peer {
    pub peer_id: Box<str>,
    pub bitfield: BitBox,

    pub status: status::Bits,
    pub conn: BufStream<TcpStream>,
}

pub(crate) mod status {
    pub type Bits = u8;

    pub const SELF_CHOKED: Bits = 1 << 0;
    pub const SELF_INTERESTED: Bits = 1 << 1;
    pub const PEER_CHOKED: Bits = 1 << 2;
    pub const PEER_INTERESTED: Bits = 1 << 3;
}

impl Peer {
    const MAX_MSG_LENGTH: u32 = 16 * 1024; // 16 KiB

    pub async fn connect(
        addr: impl ToSocketAddrs,
        info_hash: &[u8],
        peer_id: &[u8],
        total_pieces: usize,
    ) -> Option<Peer> {
        // Handshake layout:
        // length | value
        // -------+-------------------
        //      1 | 19 (hex: \x13)
        //     19 | "Bittorrent Protocol"
        //      8 | extn flags; [0u8; 8] (hex: \x00 * 8)
        //     20 | sha-1
        //     20 | peer_id
        // ------ | total
        //     68
        let mut conn = TcpStream::connect(addr).await.ok()?;
        let (mut rx, mut tx) = conn.split();

        // write our end of the handshake
        let send = async {
            const BT_PREFIX: &[u8; 28] = b"\x13Bittorrent Protocol\x00\x00\x00\x00\x00\x00\x00\x00";

            // todo: tokio docs state only the last buffer may be partially consumed, can we include
            //       an empty IoSlice and avoid manually checking if all bytes have been written?
            let mut io_bufs = &mut [
                IoSlice::new(BT_PREFIX),
                IoSlice::new(info_hash),
                IoSlice::new(peer_id),
            ][..];

            while !io_bufs.is_empty() {
                let n = tx.write_vectored(io_bufs).await?;
                IoSlice::advance_slices(&mut io_bufs, n);
            }

            Ok(())
        };

        // read a bittorrent greeting
        // allow attr bc the people working on rust can't see thee bigger picture
        #[allow(irrefutable_let_patterns)]
        let recv = async {
            const BT_PREFIX: &[u8; 20] = b"\x13Bittorrent Protocol";
            let err = Err(io::Error::from(io::ErrorKind::Other));
            let mut buf = vec![0; 20];

            // protocol prefix
            if let _ = rx.read_exact(&mut buf).await? && buf != BT_PREFIX {
                return err;
            }

            // extension flags (no extensions currently supported)
            if let _ = rx.read_exact(&mut buf[..8]).await? && buf[..8] != [0; 8] {
                return err;
            }

            // info_hash
            if let _ = rx.read_exact(&mut buf).await? && buf != info_hash {
                return err;
            }

            // peer id
            buf.fill(0);
            rx.read_exact(&mut buf).await?;
            String::from_utf8(buf).map(|s| s.into()).or(err)
        };

        let (_, peer_id) = tokio::try_join!(send, recv).ok()?;

        Some(Peer {
            status: status::SELF_CHOKED | status::PEER_CHOKED,
            bitfield: bitbox![usize, Lsb0; 0; total_pieces],
            conn: BufStream::new(conn),
            peer_id,
        })
    }

    fn peer_choked(&mut self, status: bool) {
        if status {
            self.status |= status::PEER_CHOKED;
        } else {
            self.status ^= status::PEER_CHOKED;
        }
    }

    fn peer_interested(&mut self, status: bool) {
        if status {
            self.status |= status::PEER_INTERESTED;
        } else {
            self.status ^= status::PEER_INTERESTED;
        }
    }

    fn check_msg_len(&self, id: u8, len: u32) -> bool {
        let bitfield_len = (1 + self.bitfield.len() / 8) as u32;

        match (id, len) {
            (0 | 1 | 2 | 3, 1) => true,
            (4, 5) => true,
            (5, n) if n == bitfield_len => true,
            (6 | 8, 13) => true,
            (7, n) if (9..Self::MAX_MSG_LENGTH).contains(&n) => true,
            (9, 3) => true,
            _ => false,
        }
    }

    pub async fn decode_message(&mut self) -> Result<Message, DecodeError> {
        let length = self.conn.read_u32().await?;
        if length == 0 {
            return Ok(Message::KeepAlive);
        }
        let msg_id = self.conn.read_u8().await?;

        // check msg_id matches expected message length, only Piece msgs are variable length
        if !self.check_msg_len(msg_id, length) {
            return Err(DecodeError::MessageId(msg_id, length));
        }

        let mut buf = vec![0; length as usize - 4].into_boxed_slice();
        self.conn.read_exact(&mut buf).await?;

        let mut idx = 0;

        let read_u32 = |idx: &mut usize| {
            *idx += 4;
            u32::from_be_bytes(buf[*idx - 4..*idx].try_into().unwrap())
        };

        let read_u16 = |idx: &mut usize| {
            *idx += 2;
            u16::from_be_bytes(buf[*idx - 2..*idx].try_into().unwrap())
        };

        let msg = match msg_id {
            0 => Message::Choke,
            1 => Message::Unchoke,
            2 => Message::Interested,
            3 => Message::NotInterested,
            4 => Message::Have(read_u32(&mut idx)),
            5 => Message::Bitfield(buf),
            6 => Message::Request {
                index: read_u32(&mut idx),
                begin: read_u32(&mut idx),
                length: read_u32(&mut idx),
            },
            7 => Message::Piece {
                index: read_u32(&mut idx),
                begin: read_u32(&mut idx),
                block: buf,
            },
            8 => Message::Cancel {
                index: read_u32(&mut idx),
                begin: read_u32(&mut idx),
                length: read_u32(&mut idx),
            },
            9 => Message::Port(read_u16(&mut idx)),
            _ => return Err(DecodeError::MessageId(msg_id, length)),
        };

        Ok(msg)
    }
}

pub enum Message {
    KeepAlive,                          //        | len = 0
    Choke,                              // id = 0 | len = 1
    Unchoke,                            // id = 1 | len = 1
    Interested,                         // id = 2 | len = 1
    NotInterested,                      // id = 3 | len = 1
    Have(/* piece index */ u32),        // id = 4 | len = 5
    Bitfield(/* bitfield */ Box<[u8]>), // id = 5 | len = 1+x
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
        block: Box<[u8]>,
    },
    // id = 8 | len = 13
    Cancel {
        index: u32,
        begin: u32,
        length: u32,
    },
    Port(/* listen port */ u16), // id = 9 | len = 3
}
