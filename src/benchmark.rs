extern crate byteorder;
extern crate clap;

#[macro_use]
extern crate futures;
extern crate futures_cpupool;

#[macro_use]
extern crate tokio_core;

use futures::{Async, Poll, Future};

use byteorder::{LittleEndian, ByteOrder};

use std::cell::{Cell};
use std::rc::Rc;

mod all {
    use futures::{Async, Poll, Future};
    enum ElemState<T> where T: Future {
        Pending(T),
        Done(T::Item),
    }

    pub struct All<T> where T: Future {
        elems: Vec<ElemState<T>>,
    }

    impl <T> All<T> where T: Future {
        pub fn new<I>(futures: I) -> All<T> where I: Iterator<Item=T> {
            let mut result = All { elems: Vec::new() };
            for f in futures {
                result.elems.push(ElemState::Pending(f))
            }
            result
        }
    }

    impl <T> Future for All<T> where T: Future {
        type Item = Vec<T::Item>;
        type Error = T::Error;

        fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
            let mut all_done = true;

            for idx in 0 .. self.elems.len() {
                let done_val = match &mut self.elems[idx] {
                    &mut ElemState::Pending(ref mut t) => {
                        match t.poll() {
                            Ok(Async::Ready(t)) => t,
                            Ok(Async::NotReady) => {
                                all_done = false;
                                continue
                            }
                            Err(e) => return Err(e),
                        }
                    }
                    &mut ElemState::Done(ref mut _v) => continue,
                };

                self.elems[idx] = ElemState::Done(done_val);
            }

            if all_done {
                let mut result = Vec::new();
                let elems = ::std::mem::replace(&mut self.elems, Vec::new());
                for e in elems.into_iter() {
                    match e {
                        ElemState::Done(t) => result.push(t),
                        _ => unreachable!(),
                    }
                }
                Ok(Async::Ready(result))
            } else {
                Ok(Async::NotReady)
            }
        }
    }
}

struct Knot<F, S, T, E>
    where F: Fn(S) -> T,
          T: Future<Item=(S, bool), Error=E>
{
    f: F,
    in_progress: T,
}

fn tie_knot<F, S, T, E>(initial_state: S, f: F) -> Knot<F, S, T, E>
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
        loop {
            let (s, more) = try_ready!(self.in_progress.poll());
            if more {
                self.in_progress = (self.f)(s);
            } else {
                return Ok(Async::Ready(s))
            }
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
    type Item = (R, Option<Vec<u8>>);
    type Error = ::std::io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        println!("poll reading");
        loop {
            if let Some(frame_end) = self.frame_end {
                let n = try_nb!(self.reader.as_mut().unwrap().read(&mut self.buffer[self.pos..frame_end as usize]));
                self.pos += n;
                if self.pos == frame_end as usize {
                    self.pos = 0;
                    let result = ::std::mem::replace(&mut self.buffer, Vec::new());
                    self.frame_end = None;
                    return Ok(Async::Ready((self.reader.take().unwrap(), Some(result))))
                }
            } else {
                let mut buf = [0u8];
                let n = try_nb!(self.reader.as_mut().unwrap().read(&mut buf));
                if n == 0 { // EOF
                    return Ok(Async::Ready((self.reader.take().unwrap(), None)))
                }
                println!("got length: {}", buf[0]);
                self.frame_end = Some(buf[0]);
                self.buffer = vec![0; buf[0] as usize];
            }
        }
    }
}

pub struct Writing<W> where W: ::std::io::Write {
    writer: Option<W>,
    message: Vec<u8>,
    pos: usize,
    wrote_header: bool,
}

impl <W> Writing<W> where W: ::std::io::Write {
    fn new(writer: W, message: Vec<u8>) -> Writing<W> {
        Writing {
            writer: Some(writer),
            message: message,
            pos: 0,
            wrote_header: false,
        }
    }
}

impl <W> Future for Writing<W> where W: ::std::io::Write {
    type Item = W;
    type Error = ::std::io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            if !self.wrote_header {
                let buf = [self.message.len() as u8];
                try_nb!(self.writer.as_mut().unwrap().write(&buf));
                self.wrote_header = true;
            } else {
                let n = try_nb!(self.writer.as_mut().unwrap().write(&self.message[self.pos..]));
                self.pos += n;
                if self.pos >= self.message.len() {
                    return Ok(Async::Ready(self.writer.take().unwrap()))
                }
            }
        }
    }
}

#[derive(Clone)]
struct ConnectionIdSource {
    next_id: Rc<Cell<u64>>,
}

impl ConnectionIdSource {
    fn new() -> ConnectionIdSource {
        ConnectionIdSource {
            next_id: Rc::new(Cell::new(0)),
        }
    }

    fn next(&self) -> u64 {
        let result = self.next_id.get();
        self.next_id.set(result + 1);
        result
    }
}

fn read_until_message_with_prefix<R, B>(
    reader: R,
    prefix: B)
    -> Box<Future<Item=(R, Vec<u8>), Error=::std::io::Error>>
    where R: ::std::io::Read + 'static + Send,
          B: AsRef<[u8]> + 'static + Send,
{
    Box::new(tie_knot((reader, prefix, Vec::new()), move |(reader, prefix, _)| {
        Reading::new(reader).and_then(move |(reader, message)| {
            match message {
                Some(message) => {
                    let len = prefix.as_ref().len();
                    let more = &message[0..len] != prefix.as_ref();
                    Ok(((reader, prefix, message), more))
                }
                None => {
                    Err(::std::io::Error::new(
                        ::std::io::ErrorKind::UnexpectedEof, "premature EOF"))
                }
            }
        })
    }).map(|(reader, _, message)| (reader, message)))
}

fn new_task(handle: &::tokio_core::reactor::Handle,
            addr: &::std::net::SocketAddr,
            connection_id_source: ConnectionIdSource)
            -> Box<Future<Item=(), Error=::std::io::Error> + Send>
{
    use all::All;

    let publisher = ::tokio_core::net::TcpStream::connect(addr, handle);
    let _publisher_id = connection_id_source.next();

    let mut subscribers = Vec::new();
    for _ in 0..1 {
        let subscriber_id = connection_id_source.next();
        subscribers.push(::tokio_core::net::TcpStream::connect(addr, handle).and_then(move |socket| {
            let mut buf = [0; 8];
            <LittleEndian as ByteOrder>::write_u64(&mut buf[..], subscriber_id);
            Ok(socket)
//            read_until_message_with_prefix(socket, buf).map(|(socket, _message)| {
//                socket
//            })
        }))
    }

    Box::new(publisher.join(All::new(subscribers.into_iter())).and_then(|(publisher, subscribers)| {
        println!("connected");

        tie_knot((publisher, subscribers, 10i32), |(publisher, subscribers, n)| {
            println!("looping {}", n);
            Writing::new(publisher, vec![n as u8, 1,2,3]).and_then(move |publisher| {
                All::new(subscribers.into_iter().map(|s| {
                    Reading::new(s).and_then(|(s, m)| {
                        println!("got: {:?}", m);
                        Ok(s)
                    })
                })).and_then(move |subscribers| {
                    futures::finished(((publisher, subscribers, n - 1), n > 0))
                })
            })
        }).map(|_| ())
    }))
}

pub fn run() -> Result<(), ::std::io::Error> {
    use clap::{App, Arg};
    let matches = App::new("Zillions benchmarker")
        .version("0.0.0")
        .about("Does awesome things")
        .arg(Arg::with_name("EXECUTABLE")
             .required(true)
             .index(1)
             .help("The executable to benchmark"))
        .arg(Arg::with_name("server")
             .required(false)
             .long("server")
             .short("s")
             .value_name("address")
             .default_value("127.0.0.1:8080")
             .help("address to use to connect to server"))
        .get_matches();

    let executable = matches.value_of("EXECUTABLE").unwrap();

    println!("exectuable: {}", executable);

    let addr_str = matches.value_of("server").unwrap();
    let addr = match addr_str.parse::<::std::net::SocketAddr>() {
        Ok(a) => a,
        Err(e) => {
            panic!("failed to parse socket address {}", e);
        }
    };

    let _child = ::std::process::Command::new(executable)
        .arg(addr_str)
        .stdout(::std::process::Stdio::inherit())
        .stderr(::std::process::Stdio::inherit())
        .spawn();

    // start tokio reactor
    let mut core = try!(::tokio_core::reactor::Core::new());

    let handle = core.handle();

    let pool = ::futures_cpupool::CpuPool::new_num_cpus();

    let connection_id_source = ConnectionIdSource::new();

    let f = pool.spawn(new_task(&handle, &addr, connection_id_source));

    try!(core.run(f));

    Ok(())
}

pub fn main() {
    run().expect("top level error");
}
