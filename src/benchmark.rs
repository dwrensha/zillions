extern crate byteorder;
extern crate clap;

#[macro_use]
extern crate futures;
extern crate futures_cpupool;

#[macro_use]
extern crate tokio_core;

use futures::{Async, Poll, Future};
use futures::stream::Stream;


struct Knot<F, S, T, E>
    where F: Fn(S) -> T,
          T: Future<Item=(S, bool), Error=E>
{
    f: F,
    in_progress: T,
}

fn tie_knot<F, S, T, E>(f: F, initial_state: S) -> Knot<F, S, T, E>
    where F: Fn(S) -> T,
          T: Future<Item=(S, bool), Error=E>,
{
    let in_progress = f(initial_state);
    Knot {
        f: f,
        in_progress: in_progress,
    }
}

impl <F, S, T, E> Future for Knot<F, S, T, E>
    where F: Fn(S) -> T,
          T: Future<Item=(S, bool), Error=E>
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

struct Reading<R> where R: ::std::io::Read {
    reader: Option<R>,
    buffer: Vec<u8>,
    pos: usize,
    frame_end: Option<u8>,
}

impl <R> Reading<R> where R: ::std::io::Read {
    fn new(reader: R) -> Reading<R> {
        Reading {
            reader: Some(reader),
            buffer: Vec::new(),
            pos: 0,
            frame_end: None,
        }
    }
}

impl <R> Future for Reading<R> where R: ::std::io::Read {
    type Item = (Option<Vec<u8>>, R);
    type Error = ::std::io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            if let Some(frame_end) = self.frame_end {
                let n = try_nb!(self.reader.as_mut().unwrap().read(&mut self.buffer[self.pos..frame_end as usize]));
                self.pos += n;
                if self.pos == frame_end as usize {
                    self.pos = 0;
                    let result = ::std::mem::replace(&mut self.buffer, Vec::new());
                    self.frame_end = None;
                    return Ok(Async::Ready((Some(result), self.reader.take().unwrap())))
                }
            } else {
                let mut buf = [0u8];
                let n = try_nb!(self.reader.as_mut().unwrap().read(&mut buf));
                if n == 0 { // EOF
                    return Ok(Async::Ready((None, self.reader.take().unwrap())))
                }

                self.frame_end = Some(buf[0]);
                self.buffer = vec![0; buf[0] as usize];
            }
        }
    }
}

pub struct Writing<W> {
    writer: Option<W>,
    // TODO
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
