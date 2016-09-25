#[macro_use]
extern crate futures;
extern crate tokio_core;

use std::env;
use std::net::SocketAddr;

use futures::{Async, Poll, Future};
use futures::stream::Stream;
use tokio_core::io::{read_exact, Io};
use tokio_core::net::TcpListener;
use tokio_core::reactor::Core;

struct ReadStream<R> where R: ::std::io::Read {
    reader: R,
    buffer: [u8; 256],
    pos: usize,
    frame_end: u8,
}

impl <R> Stream for ReadStream<R> where R: ::std::io::Read {
    type Item = Vec<u8>;
    type Error = ::std::io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        unimplemented!()
    }
}

pub fn main() {
    let addr = env::args().nth(1).unwrap_or("127.0.0.1:8080".to_string());
    let addr = addr.parse::<SocketAddr>().unwrap();

    let mut l = Core::new().unwrap();
    let handle = l.handle();

    // Create a TCP listener which will listen for incoming connections
    let socket = TcpListener::bind(&addr, &handle).unwrap();

    // Once we've got the TCP listener, inform that we have it
    println!("Listening on: {}", addr);

    let done = socket.incoming().for_each(move |(socket, addr)| {
        // what's the spec?
        // first byte: 0 means publisher, 1 means subscriber.

        let mut header = [0u8; 1];
        let future = read_exact(socket, header).and_then(|(socket, header)| {
            match header[0] {
                0 => {
                    // publisher
                    // hm... is this framing going to be tricky?
                    // When done receiving message, write back the number received,
                    // as a little-endian u64.
                }
                1 => {
                    //subscriber
                }
                _ => {
                    // error
                }
            }

            println!("OK {:?}", header);
            Ok(())
        }).map_err(|e| {
            println!("error: {}", e);
        });
        handle.spawn(future);

        // frame format: first byte is length of body. Then there is body.
        Ok(())
    });

    l.run(done).unwrap();
}
