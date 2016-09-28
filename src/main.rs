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

mod write_queue {
    use std::collections::VecDeque;
    use std::rc::Rc;
    use std::cell::RefCell;
    use futures::{self, task, Async, Future, Poll, Complete, Oneshot};
    use tokio_core::io::WriteAll;

    enum State<W> where W: ::std::io::Write {
        WritingHeader(W, Vec<u8>, Complete<Vec<u8>>),
        Writing(WriteAll<W, Vec<u8>>, Complete<Vec<u8>>),
        BetweenWrites(W),
        Empty,
    }

    /// A write of messages being written.
    pub struct WriteQueue<W> where W: ::std::io::Write {
        inner: Rc<RefCell<Inner>>,
        state: State<W>,
    }

    struct Inner {
        queue: VecDeque<(Vec<u8>, Complete<Vec<u8>>)>,
        sender_count: usize,
        task: Option<task::Task>,
    }

    pub struct Sender {
        inner: Rc<RefCell<Inner>>,
    }

    impl Clone for Sender {
        fn clone(&self) -> Sender {
            self.inner.borrow_mut().sender_count += 1;
            Sender { inner: self.inner.clone() }
        }
    }

    impl Drop for Sender {
        fn drop(&mut self) {
            self.inner.borrow_mut().sender_count -= 1;
        }
    }

    pub fn write_queue<W>(writer: W) -> (Sender, WriteQueue<W>)
        where W: ::std::io::Write
    {
        let inner = Rc::new(RefCell::new(Inner {
            queue: VecDeque::new(),
            task: None,
            sender_count: 1,
        }));

        let queue = WriteQueue {
            inner: inner.clone(),
            state: State::BetweenWrites(writer),
        };

        let sender = Sender { inner: inner };

        (sender, queue)
    }

    impl Sender {
        /// Enqueues a message to be written.
        pub fn send(&mut self, message: Vec<u8>) -> Oneshot<Vec<u8>> {
            let (complete, oneshot) = futures::oneshot();
            self.inner.borrow_mut().queue.push_back((message, complete));

            match self.inner.borrow_mut().task.take() {
                Some(t) => t.unpark(),
                None => (),
            }

            oneshot
        }

        /// Returns the number of messages queued to be written, not including any in-progress write.
        pub fn len(&mut self) -> usize {
            self.inner.borrow().queue.len()
        }
    }

    enum IntermediateState<W> where W: ::std::io::Write {
        WriteHeaderDone,
        WriteDone(Vec<u8>, W),
        StartWrite(Vec<u8>, Complete<Vec<u8>>),
        Resolve,
    }

    impl <W> Future for WriteQueue<W> where W: ::std::io::Write {
        type Item = W; // Resolves when all senders have been dropped and all messages written.
        type Error = ::std::io::Error;

        fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
            loop {
                let next = match self.state {
                    State::WritingHeader(ref mut write, ref buf, ref mut _complete) => {
                        let n = try_nb!(write.write(&[buf.len() as u8]));
                        match n {
                            0 => unimplemented!(), // TODO return error premature EOF
                            1 => IntermediateState::WriteHeaderDone,
                            _ => unreachable!(),
                        }
                    }
                    State::Writing(ref mut write, ref mut _complete) => {
                        let (w, m) = try_ready!(Future::poll(write));
                        IntermediateState::WriteDone(m, w)
                    }
                    State::BetweenWrites(ref mut _writer) => {
                        let front = self.inner.borrow_mut().queue.pop_front();
                        match front {
                            Some((m, complete)) => {
                                IntermediateState::StartWrite(m, complete)
                            }
                            None => {
                                let count = self.inner.borrow().sender_count;
                                if count == 0 {
                                    IntermediateState::Resolve
                                } else {
                                    self.inner.borrow_mut().task = Some(task::park());
                                    return Ok(Async::NotReady)
                                }
                            }
                        }
                    }
                    State::Empty => unreachable!(),
                };

                match next {
                    IntermediateState::WriteHeaderDone => {
                        let new_state = match ::std::mem::replace(&mut self.state, State::Empty) {
                            State::WritingHeader(writer, buf, complete) => {
                                State::Writing(::tokio_core::io::write_all(writer, buf), complete)
                            }
                            _ => unreachable!(),
                        };
                        self.state = new_state;
                    }
                    IntermediateState::WriteDone(m, w) => {
                        match ::std::mem::replace(&mut self.state, State::BetweenWrites(w)) {
                            State::Writing(_, complete) => {
                                complete.complete(m)
                            }
                            _ => unreachable!(),
                        }
                    }
                    IntermediateState::StartWrite(m, c) => {
                        let new_state = match ::std::mem::replace(&mut self.state, State::Empty) {
                            State::BetweenWrites(w) => {
                                State::WritingHeader(w, m, c)
                            }
                            _ => unreachable!(),
                        };
                        self.state = new_state;
                    }
                    IntermediateState::Resolve => {
                        match ::std::mem::replace(&mut self.state, State::Empty) {
                            State::BetweenWrites(w) => {
                                return Ok(Async::Ready(w))
                            }
                            _ => unreachable!(),
                        }
                    }
                }
            }
        }
    }
}
