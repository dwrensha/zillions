extern crate byteorder;
extern crate clap;

#[macro_use]
extern crate futures;
extern crate futures_cpupool;

#[macro_use]
extern crate tokio_core;

use futures::{Async, Poll, Future};
use futures::stream::Stream;


struct Knot<F, S, E>
    where F: Fn(S) -> Box<Future<Item=(S, bool), Error=E>>
{
    f: F,
    in_progress: Box<Future<Item=(S, bool), Error=E>>,
}

fn tie_knot<F, S, E>(f: F, initial_state: S) -> Knot<F, S, E>
    where F: Fn(S) -> Box<Future<Item=(S, bool), Error=E>>
{
    let in_progress = f(initial_state);
    Knot {
        f: f,
        in_progress: in_progress,
    }
}

impl <F, S, E> Future for Knot<F, S, E>
    where F: Fn(S) -> Box<Future<Item=(S, bool), Error=E>>
{
    type Item = S;
    type Error = E;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let (s, more) = try_ready!(self.in_progress.poll());
        if more {
            self.in_progress = (self.f)(s);
            Ok(Async::NotReady)
        } else {
            Ok(Async::Ready(s))
        }
    }
}

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
                    self.frame_end = None;
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
    use clap::{App, Arg};
    let matches = App::new("Zillions benchmarker")
        .version("0.0.0")
        .about("Does awesome things")
        .arg(Arg::with_name("EXECUTABLE")
             .required(true)
             .index(1)
             .help("The executable to benchmark"))
        .get_matches();

    let executable = matches.value_of("EXECUTABLE").unwrap();

    println!("exectuable: {}", executable);

    let addr = "127.0.0.1:8080";
    let child = ::std::process::Command::new(executable)
        .arg(addr)
        .spawn();

}
