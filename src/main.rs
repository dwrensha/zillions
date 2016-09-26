extern crate byteorder;

#[macro_use]
extern crate futures;

#[macro_use]
extern crate tokio_core;

use std::env;
use std::net::SocketAddr;
use std::rc::Rc;
use std::cell::Cell;

use byteorder::{LittleEndian, ByteOrder};
use futures::{Async, Poll, Future};
use futures::stream::Stream;
use tokio_core::io::{read_exact, write_all, Io};
use tokio_core::net::TcpListener;
use tokio_core::reactor::Core;

struct ReadStream<R> where R: ::std::io::Read {
    reader: R,
    buffer: Vec<u8>,
    pos: usize,
    frame_end: Option<u8>,
}

impl <R> ReadStream<R> where R: ::std::io::Read {
    fn new(reader: R) -> ReadStream<R> {
        ReadStream {
            reader: reader,
            buffer: Vec::new(),
            pos: 0,
            frame_end: None,
        }
    }
}

impl <R> Stream for ReadStream<R> where R: ::std::io::Read {
    type Item = Vec<u8>;
    type Error = ::std::io::Error;

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        loop {
            if let Some(frame_end) = self.frame_end {
                let n = try_nb!(self.reader.read(&mut self.buffer[self.pos..frame_end as usize]));
                self.pos += n;
                if self.pos == frame_end as usize {
                    self.pos = 0;
                    let result = ::std::mem::replace(&mut self.buffer, Vec::new());
                    return Ok(Async::Ready(Some(result)))
                }
            } else {
                let mut buf = [0u8];
                let n = try_nb!(self.reader.read(&mut buf));
                if n == 0 { // EOF
                    return Ok(Async::Ready(None))
                }

                self.frame_end = Some(buf[0]);
                self.buffer = vec![0; buf[0] as usize];
            }
        }
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

    let done = socket.incoming().for_each(move |(socket, _addr)| {
        // what's the spec?
        // first byte: 0 means publisher, 1 means subscriber.

        let header = [0u8; 1];
        let future = read_exact(socket, header).and_then(|(socket, header)| {
            println!("OK {:?}", header);
            match header[0] {
                0 => {
                    // publisher
                    let num_read = Rc::new(Cell::new(0u64));
                    let num_read1 = num_read.clone();
                    let done = futures::lazy(|| Ok(socket.split()))
                        .and_then(|(reader, writer)| {
                            ReadStream::new(reader).for_each(move |buf| {
                                num_read1.set(num_read1.get() + 1);
                                println!("buf {:?}", buf);
                                // TODO send this off to the subscribers.
                                Ok(())
                            }).and_then(move |()| {
                                let mut word = [0u8; 8];
                                <LittleEndian as ByteOrder>::write_u64(&mut word, num_read.get());
                                write_all(writer, word).map(|_| ())
                            })
                        });

                    // When this stream is done, I want the socket back.
                    done

                    // When done receiving message, write back the number received,
                    // as a little-endian u64.
                }
                1 => {
                    //subscriber
                    unimplemented!()
                }
                _ => {
                    // error
                    unimplemented!()
                }
            }
        }).map_err(|e| {
            println!("error: {}", e);
        });
        handle.spawn(future);

        // frame format: first byte is length of body. Then there is body.
        Ok(())
    });

    l.run(done).unwrap();
}
