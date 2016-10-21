use std::env;
use std::net::SocketAddr;
use std::io::{Read, Write, BufRead};

fn run() -> Result<(), ::std::io::Error> {
    let addr = env::args().nth(1).unwrap_or("127.0.0.1:8080".to_string());
    let addr = addr.parse::<SocketAddr>().unwrap();

    let mut socket = try!(::std::net::TcpStream::connect(addr));

    let mut read_stream = try!(socket.try_clone());

    let read_thread = ::std::thread::spawn(move || {
        let mut message = [0u8; 256];
        let mut header = [0u8];
        loop {
            if read_stream.read(&mut header).unwrap() == 0 {
                // EOF
                break;
            }

            let len = header[0] as usize;
            read_stream.read_exact(&mut message[..len]).unwrap();
            match ::std::str::from_utf8(&message[..len]) {
                Ok(s) => {
                    println!("{}", s);
                }
                Err(_) => {
                    println!("[received non-utf8 data]");
                }
            }
        }
    });

    socket.write_all(&[0]).unwrap();

    let stdin = std::io::stdin();
    let handle = stdin.lock();
    for line in handle.lines() {
        match line {
            Ok(l) => {
                let bytes = l.as_bytes();
                if bytes.len() <= 255 {
                    socket.write_all(&[bytes.len() as u8]).unwrap();
                    socket.write_all(bytes).unwrap();
                } else {
                    unimplemented!();
                }
            }
            Err(_) => unimplemented!(),
        }
    }

    read_thread.join().expect("read thread join");

    Ok(())
}

pub fn main() {
    run().expect("top level");
}
