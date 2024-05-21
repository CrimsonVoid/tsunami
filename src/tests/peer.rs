use std::mem::{size_of, size_of_val};

use tokio::{
    io::BufStream,
    net::{TcpListener, TcpStream},
};

use crate::peer::Peer;

#[allow(dead_code)]
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
        peer_id: "".into(),
        bitfield: Default::default(),
        status: 0,
        conn: BufStream::new(TcpStream::connect(addr).await.unwrap()),
    };

    println!(
        "connect: {} bytes",
        size_of_val(&Peer::connect(addr, &b""[..], &b""[..], 0))
    );

    println!(
        "decode_message baseline is {:?} bytes",
        size_of::<MsgData>(),
    );

    println!("decode_message: {} bytes", size_of_val(&p.decode_message()));
}
