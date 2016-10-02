extern crate byteorder;

use std::env;
use std::net::SocketAddr;
use std::io::{Read, Write, BufRead};

use byteorder::{LittleEndian, ByteOrder};

pub fn main() {
    let addr = env::args().nth(1).unwrap_or("127.0.0.1:8080".to_string());
    let addr = addr.parse::<SocketAddr>().unwrap();

    let mut socket = ::std::net::TcpStream::connect(addr).unwrap();

    socket.write_all(&[0]).unwrap();

    let mut message_count = 0;

    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    for line in handle.lines() {
        match line {
            Ok(l) => {
                let bytes = l.as_bytes();
                if bytes.len() <= 255 {
                    socket.write_all(&[bytes.len() as u8]).unwrap();
                    socket.write_all(bytes).unwrap();

                    message_count += 1;
                } else {
                    unimplemented!();
                }
            }
            Err(_) => unimplemented!(),
        }
    }
    socket.shutdown(std::net::Shutdown::Write).unwrap();

    let mut word = [0u8; 8];
    socket.read_exact(&mut word).unwrap();

    let reply = <LittleEndian as ByteOrder>::read_u64(&word);
    println!("reply = {}", reply);
}
