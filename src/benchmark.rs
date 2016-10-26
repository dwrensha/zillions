extern crate byteorder;
extern crate clap;

#[macro_use]
extern crate futures;
extern crate futures_cpupool;

#[macro_use]
extern crate tokio_core;

use futures::{Async, Poll, Future, Complete};

use byteorder::{LittleEndian, ByteOrder};

use std::cell::{Cell};
use std::rc::Rc;
use std::time::Duration;

macro_rules! fry {
    ($expr:expr) => (
        match $expr {
            ::std::result::Result::Ok(val) => val,
            ::std::result::Result::Err(err) => {
                return Box::new(::futures::failed(::std::convert::From::from(err)))
            }
        })
}

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

struct ReadTaskWaitFor {
    prefix: Vec<u8>,
    complete: Complete<Vec<u8>>,
    timeout_ticks: u64,
}

type ChannelElem = ReadTaskWaitFor;

struct ReadTask<R> where R: ::std::io::Read {
    in_progress: Reading<R>,
    number_successfully_read: u64,
    receiver: ::tokio_core::channel::Receiver<ChannelElem>,
    waiting_for: Option<ReadTaskWaitFor>
}

impl <R> ReadTask<R> where R: ::std::io::Read {
    fn new(reader: R, receiver: ::tokio_core::channel::Receiver<ChannelElem>)
           -> ReadTask<R> {
        ReadTask {
            in_progress: Reading::new(reader),
            number_successfully_read: 0,
            receiver: receiver,
            waiting_for: None,
        }
    }
}

impl <R> Future for ReadTask<R> where R: ::std::io::Read {
    type Item = u64; // Total number of matching messages received.
    type Error = ::std::io::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        use futures::stream::Stream;

        loop {
            if self.waiting_for.is_none() {
                match self.receiver.poll() {
                    Ok(Async::Ready(Some(wait_for))) => {
                        self.waiting_for = Some(wait_for);
                    }
                    Ok(Async::Ready(None)) => {
                        return Ok(Async::Ready(self.number_successfully_read))
                    }
                    Ok(Async::NotReady) => (),
                    Err(e) => {
                        return Err(e)
                    }
                }
            }

            let (reader, message) = try_ready!(self.in_progress.poll());
            self.in_progress = Reading::new(reader);
            match message {
                Some(message) => {
                    enum A {
                        Matches,
                        TimedOut,
                    }
                    let aa = match self.waiting_for {
                        Some(ReadTaskWaitFor { ref prefix, ref mut timeout_ticks, .. }) => {
                            let len = prefix.len();
                            if &message[0..len] == &prefix[..] {
                                Some(A::Matches)
                            } else if &message[..CLOCK_PREFIX.len()] == CLOCK_PREFIX {
                                if *timeout_ticks == 0 {
                                    // timed out.
                                    Some(A::TimedOut)
                                } else {
                                    *timeout_ticks -= 1;
                                    None
                                }
                            } else {
                                None
                            }
                        }
                        None => None,
                    };
                    match aa {
                        Some(A::Matches) => {
                            self.number_successfully_read += 1;
                            let waiting_for = self.waiting_for.take().unwrap();
                            waiting_for.complete.complete(message);
                        }
                        Some(A::TimedOut) => {
                            self.waiting_for.take();
                        }
                        None => (),
                    }
                }
                None => {
                    return Ok(Async::Ready(self.number_successfully_read))
                }
            }
        }
    }
}

fn initialize_subscribers(
    handle: &::tokio_core::reactor::Handle,
    _pool: &::futures_cpupool::CpuPool,
    addr: &::std::net::SocketAddr,
    connection_id_source: ConnectionIdSource,
    number_of_subscribers: u64)
    -> Result<(Box<Future<Item=u64, Error=::std::io::Error> + Send>,
               Box<Future<Item=Vec<::tokio_core::channel::Sender<ChannelElem>>,
                          Error=::std::io::Error>>),
              ::std::io::Error>
{
    use all::All;

    let mut subscriber_read_tasks = Vec::new();
    let mut subscriber_senders: Option<Box<Future<Item=Vec<::tokio_core::channel::Sender<ChannelElem>>,
                                                  Error=::std::io::Error> + Send>> =
        Some(Box::new(futures::finished(Vec::new())));
    for _ in 0..number_of_subscribers {
        let subscriber_id = connection_id_source.next();

        let (sender, receiver) = try!(::tokio_core::channel::channel(handle));
        let (sender_complete, sender_oneshot) = ::futures::oneshot();

        let subscriber_senders1 = subscriber_senders.take().unwrap();
        subscriber_senders = Some(Box::new(sender_oneshot.map_err(|_| {
            ::std::io::Error::new(::std::io::ErrorKind::Other,"canceled")
        })));

        subscriber_read_tasks.push(::tokio_core::net::TcpStream::connect(addr, handle).and_then(move |socket| {
            subscriber_senders1.and_then(move |mut subscriber_senders|  {
                use tokio_core::io::Io;
                let (reader, writer) = socket.split();
                let read_task = ReadTask::new(reader, receiver);

                let sender_init = futures::finished(()).and_then(move |()| {
                    let mut buf = vec![0; 8];
                    <LittleEndian as ByteOrder>::write_u64(&mut buf[..], subscriber_id);
                    let writing = Writing::new(writer, buf.clone());

                    let (complete, oneshot) = ::futures::oneshot();

                    let wait_for = ReadTaskWaitFor {
                        prefix: buf,
                        complete: complete,
                        timeout_ticks: 2,
                    };

                    try!(sender.send(wait_for));

                    Ok((oneshot, writing, sender))
                }).and_then(move |(oneshot, writing, sender)| {
                    oneshot.then(move |r| match r {
                        Ok(_) => Ok(true),
                        Err(::futures::Canceled) => Ok(false)
                    }).join(writing).map(move |(succeeded, _writer)| {
                        if succeeded {
                            subscriber_senders.push(sender)
                        }
                        sender_complete.complete(subscriber_senders);
                    })
                });

                read_task.join(sender_init)
            }).map(|(n, _)| n)
        }));
    }

    let read_tasks = All::new(subscriber_read_tasks.into_iter()).and_then(move |read_values| {
        let mut sum = 0;
        for idx in 0..read_values.len() {
            sum += read_values[idx];
        }
        Ok(sum)
    });

    Ok((Box::new(read_tasks), subscriber_senders.unwrap()))
}

fn run_publisher(
    handle: &::tokio_core::reactor::Handle,
    _pool: &::futures_cpupool::CpuPool,
    addr: &::std::net::SocketAddr,
    connection_id_source: ConnectionIdSource,
    number_of_messages: u64,
    senders: Vec<::tokio_core::channel::Sender<ChannelElem>>)
    -> Box<Future<Item=(), Error=::std::io::Error>>
{
    let publisher = ::tokio_core::net::TcpStream::connect(addr, handle);
    let publisher_id = connection_id_source.next();

    Box::new(publisher.and_then(move |publisher| {
        tie_knot((publisher, senders, 0u64), move |(publisher, senders, n)| {
            ::futures::finished(()).and_then(move |()| {
                let mut buf = vec![255; 16];
                let mut prefix = vec![0; 8];
                <LittleEndian as ByteOrder>::write_u64(&mut buf[..8], publisher_id);
                <LittleEndian as ByteOrder>::write_u64(&mut prefix[..], publisher_id);

                let mut dones = Vec::new();
                for idx in 0..senders.len() {
                    let (complete, oneshot) = ::futures::oneshot();
                    dones.push(oneshot);
                    let wait_for = ReadTaskWaitFor {
                        prefix: prefix.clone(),
                        complete: complete,
                        timeout_ticks: 2,
                    };
                    try!(senders[idx].send(wait_for))
                }

                let writing = Writing::new(publisher, buf);
                Ok((dones, writing, senders))
            }).and_then(move |(dones, writing, senders)| {
                use all::All;
                let done = All::new(dones.into_iter()).then(|r| match r {
                    Ok(_) => Ok(true),
                    Err(_) => Ok(false),
                });

                writing.join(done).map(move |(writer, _done)| {
                    ((writer, senders, n + 1), n + 1 < number_of_messages)
                })
            })
        })
    }).map(|(_writer, _senders, _n)| ()))
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
    let matches = App::new("Zillions stress tester")
        .version("0.0.0")
        .about("Runs a given zillions chat server and connects some automated clients to it.")
        .arg(Arg::with_name("EXECUTABLE")
             .required(true)
             .index(1)
             .help("The executable to benchmark"))
        .arg(Arg::with_name("server")
             .required(false)
             .long("server")
             .short("a")
             .value_name("address")
             .default_value("127.0.0.1:8080")
             .help("address to use to connect to server"))
        .arg(Arg::with_name("publishers")
             .required(false)
             .long("publishers")
             .short("p")
             .value_name("count")
             .default_value("2")
             .help("number of publishers to start"))
        .arg(Arg::with_name("subscribers")
             .required(false)
             .long("subscribers")
             .short("s")
             .value_name("count")
             .default_value("3")
             .help("number of publishers to start for each publisher"))
        .arg(Arg::with_name("messages")
             .required(false)
             .long("messages")
             .short("m")
             .value_name("count")
             .default_value("1000")
             .help("number of messages to send from each publisher"))
        .get_matches();

    let number_of_publishers = matches.value_of("publishers").unwrap().parse::<u64>()
        .expect("parsing 'publishers'");
    let number_of_subscribers = matches.value_of("subscribers").unwrap().parse::<u64>()
        .expect("parsing 'subscribers'");
    let number_of_messages = matches.value_of("messages").unwrap().parse::<u64>()
        .expect("parsing 'messages'");
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


    let mut init_futures = Vec::new();
    let mut read_tasks = Vec::new();
    for _ in 0..number_of_publishers {
        let (number_read, senders) =
            try!(initialize_subscribers(
                &handle, &pool, &addr, connection_id_source.clone(), number_of_subscribers));
        init_futures.push(senders);
        read_tasks.push(number_read);
    }

    let read_tasks = ::all::All::new(read_tasks.into_iter()).map(|num_reads| {
        let mut sum = 0;
        for idx in 0..num_reads.len() {
            sum += num_reads[idx];
        }
        sum
    });

    let handle1 = handle.clone();
    let pool1 = pool.clone();
    let write_tasks = ::all::All::new(init_futures.into_iter()).and_then(move |ss| {
        let mut publishers = Vec::new();
        for senders in ss.into_iter() {
            publishers.push(
                run_publisher(&handle1, &pool1, &addr, connection_id_source.clone(), number_of_messages, senders));
        }
        ::all::All::new(publishers.into_iter())
    });

    let read_tasks = pool.spawn(read_tasks);

    let x = try!(core.run(read_tasks.join(write_tasks)));
    println!("x = {:?}", x);

    Ok(())
}

pub fn main() {
    run().expect("top level error");
}
