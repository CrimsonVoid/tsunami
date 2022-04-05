use std::io::IoSlice;

use bitflags::bitflags;
use bitvec::prelude::{bitbox, BitBox, Lsb0};
use byteorder::{ByteOrder, BE};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufStream},
    net::{TcpStream, ToSocketAddrs},
};

use crate::error::{DecodeError, Result};

#[derive(Debug)]
crate struct Peer {
    peer_id: String,
    bitfield: BitBox,

    status: Status,
    conn: BufStream<TcpStream>,
}

bitflags! {
    struct Status: u8 {
        const SELF_CHOKED = 1 << 0;
        const SELF_INTERESTED = 1 << 1;
        const PEER_CHOKED = 1 << 2;
        const PEER_INTERESTED = 1 << 3;
    }
}

impl Peer {
    const MAX_MSG_LENGTH: u32 = 1024 * 16; // 16 KiB

    crate async fn connect(
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
            let prefix = b"\x13Bittorrent Protocol\x00\x00\x00\x00\x00\x00\x00\x00";

            // todo: tokio docs state only the last buffer may be partially consumed, can we include
            //       an empty IoSlice and avoid manually checking if all bytes have been written?
            let mut io_bufs = &mut [
                IoSlice::new(prefix),
                IoSlice::new(info_hash),
                IoSlice::new(peer_id),
            ][..];

            while io_bufs.len() != 0 {
                let n = tx.write_vectored(&io_bufs).await?;
                IoSlice::advance_slices(&mut io_bufs, n);
            }

            Ok(())
        };

        // read a bittorrent greeting
        let recv = async {
            let err = Err(std::io::Error::from(std::io::ErrorKind::Other));
            let mut buffer = vec![0; 20];

            // protocol prefix
            rx.read_exact(&mut buffer[..19]).await?;
            if buffer[..19] != *b"\x13Bittorrent Protocol" {
                return err;
            }

            // extension flags
            rx.read_exact(&mut buffer[..8]).await?;
            if &buffer[..8] != [0; 8] {
                // we currently do not support any bt extensions
                return err;
            }

            // info_hash
            rx.read_exact(&mut buffer).await?;
            if buffer != info_hash {
                return err;
            }

            // peer id
            buffer.fill(0);
            rx.read_exact(&mut buffer).await?;
            String::from_utf8(buffer).or(err)
        };

        let (_, peer_id) = futures::try_join!(send, recv).ok()?;

        Some(Peer {
            status: Status::SELF_CHOKED | Status::PEER_CHOKED,
            bitfield: bitbox![usize, Lsb0; 0; total_pieces],
            conn: BufStream::new(conn),
            peer_id,
        })
    }

    fn peer_choked(&mut self, status: bool) {
        self.status.set(Status::PEER_CHOKED, status);
    }

    fn peer_interested(&mut self, status: bool) {
        self.status.set(Status::PEER_INTERESTED, status);
    }

    fn check_msg_len(&self, id: u8, len: u32) -> bool {
        let bitfield_len = (1 + self.bitfield.len() / 8) as u32;

        match (id, len) {
            (0 | 1 | 2 | 3, 1) => true,
            (4, 5) => true,
            (5, n) if n == bitfield_len => true,
            (6 | 8, 13) => true,
            (7, n) if n >= 9 && n < Self::MAX_MSG_LENGTH => true,
            (9, 3) => true,
            _ => false,
        }
    }

    async fn decode_message(&mut self) -> Result<Message, DecodeError> {
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

        let msg = match msg_id {
            0 => Message::Choke,
            1 => Message::Unchoke,
            2 => Message::Interested,
            3 => Message::NotInterested,
            4 => Message::Have(BE::read_u32(&buf[..])),
            5 => Message::Bitfield(buf),
            6 => Message::Request {
                index: BE::read_u32(&buf[..]),
                begin: BE::read_u32(&buf[..]),
                length: BE::read_u32(&buf[..]),
            },
            7 => Message::Piece {
                index: BE::read_u32(&buf[..]),
                begin: BE::read_u32(&buf[..]),
                block: buf,
            },
            8 => Message::Cancel {
                index: BE::read_u32(&buf[..]),
                begin: BE::read_u32(&buf[..]),
                length: BE::read_u32(&buf[..]),
            },
            9 => Message::Port(BE::read_u16(&buf[..])),
            _ => return Err(DecodeError::MessageId(msg_id, length)),
        };

        Ok(msg)
    }
}

crate enum Message {
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

#[cfg(test)]
mod test {
    use std::mem::{size_of, size_of_val};

    use tokio::{
        io::BufStream,
        net::{TcpListener, TcpStream},
    };

    use crate::peer::{Peer, Status};

    struct MsgData {
        length: u32,
        msg_id: u8,
        buf: [u8; 13],
        block: Box<[u8]>,
    }

    #[tokio::test]
    async fn arr_size() {
        let addr = "127.0.0.1:34567";
        let _l = TcpListener::bind(addr).await.unwrap();

        let mut p = Peer {
            peer_id: "".to_string(),
            bitfield: Default::default(),
            status: Status { bits: 0 },
            conn: BufStream::new(TcpStream::connect(addr).await.unwrap()),
        };

        println!(
            "handshake future is {:?} bytes",
            size_of_val(&Peer::connect(addr, &b""[..], &b""[..], 0))
        );

        println!(
            "decode_message baseline is {:?} bytes",
            size_of::<MsgData>(),
        );

        println!(
            "decode_message future is {:?} bytes",
            size_of_val(&p.decode_message())
        );
    }
}
