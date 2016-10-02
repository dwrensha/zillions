use std::env;
use std::net::SocketAddr;
use std::io::{Read, Write, BufRead};

pub fn main() {
    let addr = env::args().nth(1).unwrap_or("127.0.0.1:8080".to_string());
    let addr = addr.parse::<SocketAddr>().unwrap();

    let mut socket = ::std::net::TcpStream::connect(addr).unwrap();

    socket.write_all(&[1]).unwrap(); // subscribe

    // TODO

}
