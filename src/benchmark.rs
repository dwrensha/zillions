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
use std::time::Duration;

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
                self.frame_end = Some(buf[0]);
                self.buffer = vec![0; buf[0] as usize];
            }
        }
    }
}

pub struct Writing<W, B> where W: ::std::io::Write, B: AsRef<[u8]> {
    writer: Option<W>,
    message: B,
    pos: usize,
    wrote_header: bool,
}

impl <W, B> Writing<W, B> where W: ::std::io::Write, B: AsRef<[u8]> {
    fn new(writer: W, message: B) -> Writing<W, B> {
        Writing {
            writer: Some(writer),
            message: message,
            pos: 0,
            wrote_header: false,
        }
    }
}

impl <W, B> Future for Writing<W, B> where W: ::std::io::Write, B: AsRef<[u8]> {
    type Item = W;
    type Error = ::std::io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        loop {
            if !self.wrote_header {
                let buf = [self.message.as_ref().len() as u8];
                try_nb!(self.writer.as_mut().unwrap().write(&buf));
                self.wrote_header = true;
            } else {
                let n = try_nb!(self.writer.as_mut().unwrap().write(&self.message.as_ref()[self.pos..]));
                self.pos += n;
                if self.pos >= self.message.as_ref().len() {
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
            next_id: Rc::new(Cell::new(1)),
        }
    }

    fn next(&self) -> u64 {
        let result = self.next_id.get();
        self.next_id.set(result + 1);
        result
    }
}

static CLOCK_PREFIX: &'static [u8] = &[0,0,0,0,0,0,0,0];

fn read_until_message_with_prefix<R, B>(
    reader: R,
    prefix: B,
    timeout_seconds: u64)
    -> Box<Future<Item=(R, Option<Vec<u8>>), Error=::std::io::Error> + 'static + Send>
    where R: ::std::io::Read + 'static + Send,
          B: AsRef<[u8]> + 'static + Send,
{
    Box::new(tie_knot((reader, prefix, 0, None), move |(reader, prefix, ticks, _)| {
        Reading::new(reader).and_then(move |(reader, message)| {
            match message {
                Some(message) => {
                    let len = prefix.as_ref().len();
                    let mut new_ticks = ticks;
                    if  &message[0..len] != prefix.as_ref() {
                        Ok(((reader, prefix, new_ticks, Some(message)), false))
                    } else {
                        if &message[..CLOCK_PREFIX.len()] == CLOCK_PREFIX {
                            new_ticks = ticks + 1;
                        }

                        if new_ticks > timeout_seconds {
                            Ok(((reader, prefix, new_ticks, None), false))
                        }  else {
                            Ok(((reader, prefix, new_ticks, Some(message)), true))
                        }
                    }
                }
                None => {
                    Ok(((reader, prefix, ticks, None), false))
                }
            }
        })
    }).map(|(reader, _, _, message)| (reader, message)))
}

fn read_until_eof<R>(reader: R)
                     -> Box<Future<Item=R, Error=::std::io::Error> + 'static + Send>
    where R: ::std::io::Read + 'static + Send,
{
    Box::new(tie_knot(reader, move |reader| {
        Reading::new(reader).and_then(move |(reader, message)| {
            match message {
                Some(_) => {
                    Ok((reader, true))
                }
                None => {
                    Ok((reader, false))
                }
            }
        })
    }))
}

fn new_task(handle: &::tokio_core::reactor::Handle,
            addr: &::std::net::SocketAddr,
            connection_id_source: ConnectionIdSource,
            number_of_messages: u64)
            -> Box<Future<Item=usize, Error=::std::io::Error> + Send>
{
    use all::All;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let publisher = ::tokio_core::net::TcpStream::connect(addr, handle);
    let publisher_id = connection_id_source.next();

    let mut subscribers = Vec::new();
    for _ in 0..2 {
        let subscriber_id = connection_id_source.next();
        subscribers.push(::tokio_core::net::TcpStream::connect(addr, handle).and_then(move |socket| {
            let mut buf = [0; 8];
            <LittleEndian as ByteOrder>::write_u64(&mut buf[..], subscriber_id);
            Writing::new(socket, buf).and_then(move |socket| {
                read_until_message_with_prefix(socket, buf, 2)
                    .map(|(socket, _message)| { socket })
            })
        }))
    }

    Box::new(publisher.join(All::new(subscribers.into_iter())).and_then(move |(publisher, subscribers)| {
        let successful_message_count = ::futures::task::TaskRc::new(AtomicUsize::new(0));
        let smc = successful_message_count.clone();
        tie_knot((publisher, subscribers, 0u64), move |(publisher, subscribers, n)| {
            println!("looping {}", n);
            let mut buf = vec![255; 16];
            let mut prefix = [0; 8];
            <LittleEndian as ByteOrder>::write_u64(&mut buf[..8], publisher_id);
            <LittleEndian as ByteOrder>::write_u64(&mut prefix[..], publisher_id);

            let smc = smc.clone();
            Writing::new(publisher, buf).and_then(move |publisher| {
                All::new(subscribers.into_iter().map(move |s| {
                    let smc = smc.clone();
                    read_until_message_with_prefix(s, prefix, 2)
                        .map(move |(stream, message)| {
                            match message {
                                Some(_) => {
                                    // TODO check that the message is what the publisher sent.
                                    smc.with(|x| {
                                        x.fetch_add(1, Ordering::SeqCst);
                                    });
                                }
                                None => (),
                            }
                            stream
                        })
                })).and_then(move |subscribers| {
                    futures::finished(((publisher, subscribers, n + 1), n < number_of_messages))
                })
            })
        }).and_then(|(publisher, subscribers, _)| {
            try!(publisher.shutdown(::std::net::Shutdown::Write));
            for sub in &subscribers {
                try!(sub.shutdown(::std::net::Shutdown::Write));
            }
            Ok((publisher, subscribers))
        }).and_then(|(publisher, subscribers)| {
            read_until_eof(publisher).join(
                All::new(subscribers.into_iter().map(|sub| {
                    read_until_eof(sub)
                })))
        }).map(move |_| successful_message_count.with(|x| x.load(Ordering::SeqCst)))
    }))
}

struct ChildProcess {
    child: ::std::process::Child
}

impl ChildProcess {
    fn new(child: ::std::process::Child) -> ChildProcess {
        ChildProcess { child: child }
    }
}

impl Drop for ChildProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
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

    let mut child = try!(::std::process::Command::new(executable)
        .arg(addr_str)
        .stdout(::std::process::Stdio::piped())
        .stderr(::std::process::Stdio::inherit())
        .spawn());

    let mut child_stdout = ::std::io::BufReader::new(child.stdout.take().unwrap());
    let _wrapped_child = ChildProcess::new(child);

    let mut first_line = String::new();
    try!(::std::io::BufRead::read_line(&mut child_stdout, &mut first_line));

    if !first_line.starts_with("listening on") {
        return Err(::std::io::Error::new(
            ::std::io::ErrorKind::Other,
            format!(
                "expected first line from server to start with 'listening on ', but got {}",
                first_line)))
    }

    // start tokio reactor
    let mut core = try!(::tokio_core::reactor::Core::new());

    let handle = core.handle();

    let pool = ::futures_cpupool::CpuPool::new_num_cpus();
    let connection_id_source = ConnectionIdSource::new();

    // Start a connection whose sole job is to send periodic "tick" messages.
    let clock_connection = ::tokio_core::net::TcpStream::connect(&addr, &handle);
    let handle1 = handle.clone();
    handle.spawn(clock_connection.and_then(move |stream| {
        tie_knot((stream, handle1), move |(stream, handle)| {
            use tokio_core::reactor::Timeout;
            Timeout::new(Duration::from_secs(1), &handle).expect("creating timeout").and_then(move |()| {
                Writing::new(stream, CLOCK_PREFIX).map(move |stream| {
                    ((stream, handle), true)
                })
            })
        })
    }).map(|_| ()).map_err(|e| { println!("error from clock task: {}", e); () }));

    let f = pool.spawn(new_task(&handle, &addr, connection_id_source.clone(), 5)
                       .join(new_task(&core.handle(), &addr, connection_id_source, 5)));

    let x = try!(core.run(f));
    println!("x = {:?}", x);

    Ok(())
}

pub fn main() {
    run().expect("top level error");
}
