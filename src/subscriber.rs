use std::env;
use std::net::SocketAddr;
use std::io::{Read, Write};

pub fn main() {
    let addr = env::args().nth(1).unwrap_or("127.0.0.1:8080".to_string());
    let addr = addr.parse::<SocketAddr>().unwrap();

    let mut socket = ::std::net::TcpStream::connect(addr).unwrap();

    socket.write_all(&[1]).unwrap(); // subscribe

    let mut message = [0u8; 256];
    let mut header = [0u8];

    loop {
        if socket.read(&mut header).unwrap() == 0 {
            // EOF
            break;
        }

        let len = header[0] as usize;
        socket.read_exact(&mut message[..len]).unwrap();
        match ::std::str::from_utf8(&message[..len]) {
            Ok(s) => {
                println!("{}", s);
            }
            Err(_) => {
                println!("[received non-utf8 data]");
            }
        }
    }
}
