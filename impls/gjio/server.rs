extern crate gj;
extern crate gjio;
extern crate slab;

use std::io::{Error, ErrorKind};
use std::rc::{Rc, Weak};
use std::cell::{Cell, RefCell};
use slab::Slab;
use gj::{EventLoop, Promise, TaskReaper, TaskSet};
use gjio::{SocketStream, AsyncRead, AsyncWrite};

struct WriteQueue {
    task: Promise<(SocketStream, Bomb), Error>,
    len: Rc<Cell<usize>>,
}

impl WriteQueue {
    fn new() -> WriteQueue {
        WriteQueue {
            task: Promise::err(Error::new(ErrorKind::Other, "uninitialized")),
            len: Rc::new(Cell::new(0)),
        }
    }

    fn init(&mut self, idx: usize, subscribers: &Rc<RefCell<Slab<WriteQueue>>>,
            stream: SocketStream ) {
        self.task = Promise::ok((stream, Bomb {
            subscribers: Rc::downgrade(subscribers),
            idx: idx
        }));
    }

    fn len(&self) -> usize {
        self.len.get()
    }

    fn send(&mut self, message: Vec<u8>) {
        let task = ::std::mem::replace(&mut self.task, Promise::err(Error::new(ErrorKind::Other, "uninitialized")));
        self.len.set(self.len.get() + 1);
        let len = self.len.clone();
        self.task = task.then(move |(mut stream, bomb)| {
            let header = vec![message.len() as u8];
            stream.write(header).then(move |_| {
                stream.write(message).then(move |_| {
                    len.set(len.get() - 1);
                    Promise::ok((stream, bomb))
                })
            })
        });
    }
}

struct Bomb {
    subscribers: Weak<RefCell<Slab<WriteQueue>>>,
    idx: usize,
}

impl Drop for Bomb {
    fn drop(&mut self) {
        match self.subscribers.upgrade() {
            Some(s) => {
                s.borrow_mut().remove(self.idx).unwrap();
            }
            None => (),
        }
    }
}

fn handle_publisher(mut stream: SocketStream, messages_received: u64,
                    subscribers: Rc<RefCell<Slab<WriteQueue>>>) -> Promise<(), Error> {
    stream.try_read(vec![0], 1).then(move |(buf, n)| {
        if n == 0 {
            // EOF
            Promise::ok(())
        } else {
            let len = buf[0] as usize;
            let body = vec![0u8; len];
            stream.read(body, len).then(move |(buf, _)| {
                for ref mut write_queue in subscribers.borrow_mut().iter_mut() {
                    if write_queue.len() < 5 {
                        write_queue.send(buf.clone());
                    }
                }

                handle_publisher(stream, messages_received + 1, subscribers)
            })
        }
    })
}

fn handle_connection(stream: SocketStream,
                     subscribers: Rc<RefCell<Slab<WriteQueue>>>)
                     -> Promise<(), Error> {
    let read_stream = stream.clone();

    let write_queue = WriteQueue::new();
    if !subscribers.borrow().has_available() {
        let len = subscribers.borrow().len();
        subscribers.borrow_mut().reserve_exact(len);
    }
    let idx = match subscribers.borrow_mut().insert(write_queue) {
        Ok(idx) => idx,
        Err(_) => unreachable!(),
    };

    match subscribers.borrow_mut().get_mut(idx) {
        Some(ref mut q) => q.init(idx, &subscribers, stream),
        None => unreachable!(),
    }

    handle_publisher(read_stream, 0, subscribers)
}

fn accept_loop(listener: gjio::SocketListener,
               mut task_set: TaskSet<(), ::std::io::Error>,
               subscribers: Rc<RefCell<Slab<WriteQueue>>>)
               -> Promise<(), ::std::io::Error>
{
     listener.accept().then(move |stream| {
         task_set.add(handle_connection(stream, subscribers.clone()));
         accept_loop(listener, task_set, subscribers)
    })
}

struct Reaper;

impl TaskReaper<(), ::std::io::Error> for Reaper {
    fn task_failed(&mut self, error: ::std::io::Error) {
        println!("Task failed: {}", error);
    }
}

pub fn main() {
    let args: Vec<String> = ::std::env::args().collect();
    if args.len() != 2 {
        println!("usage: {} HOST:PORT", args[0]);
        return;
    }

    EventLoop::top_level(move |wait_scope| -> Result<(), Box<::std::error::Error>> {
        let mut event_port = try!(gjio::EventPort::new());
        let network = event_port.get_network();
        let addr_str = &args[1];
        let addr = try!(addr_str.parse::<::std::net::SocketAddr>());
        let mut address = network.get_tcp_address(addr);
        let listener = try!(address.listen());
        println!("listening on {}", addr_str);

        let reaper = Box::new(Reaper);

        let subscribers: Rc<RefCell<Slab<WriteQueue>>> =
            Rc::new(RefCell::new(Slab::with_capacity(1024)));

        try!(accept_loop(listener, TaskSet::new(reaper), subscribers).wait(wait_scope, &mut event_port));
        Ok(())
    }).expect("top level");

}
