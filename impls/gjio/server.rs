extern crate gj;
extern crate gjio;
extern crate byteorder;
extern crate slab;

use std::io::{Error, ErrorKind};
use byteorder::{LittleEndian, ByteOrder};
use gj::{EventLoop, Promise, TaskReaper, TaskSet};
use gjio::{SocketStream, AsyncRead, AsyncWrite};

fn handle_publisher(mut stream: SocketStream, messages_received: u64) -> Promise<(), Error> {
    stream.try_read(vec![0], 1).then(move |(buf, n)| {
        if n == 0 {
            // EOF
            let mut word = vec![0u8; 8];
            <LittleEndian as ByteOrder>::write_u64(&mut word, messages_received);
            stream.write(word).map(|_| Ok(()))
        } else {
            let len = buf[0] as usize;
            let body = vec![0u8; len];
            stream.read(body, len).then(move |(buf, _)| {
                // TODO send buf to subscribers
                handle_publisher(stream, messages_received + 1)
            })
        }
    })
}


fn handle_connection(mut stream: SocketStream) -> Promise<(), Error> {
    stream.read(vec![0], 1).then(move |(buf, _)| {
        match buf[0] {
            0 => {
                // publisher
                handle_publisher(stream, 0)
            }
            1 => {
                // subscriber
                unimplemented!()
            }
            _ => {
                Promise::err(Error::new(ErrorKind::Other, "expected 0 or 1"))
            }
        }
    })
}

fn accept_loop(listener: gjio::SocketListener,
               mut task_set: TaskSet<(), ::std::io::Error>)
               -> Promise<(), ::std::io::Error>
{
     listener.accept().then(move |stream| {
         task_set.add(handle_connection(stream));
         accept_loop(listener, task_set)
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

    EventLoop::top_level(move |wait_scope| -> Result<(), ::std::io::Error> {
        use std::net::ToSocketAddrs;
        let mut event_port = try!(gjio::EventPort::new());
        let network = event_port.get_network();
        let addr = try!(args[1].to_socket_addrs()).next().expect("could not parse address");
        let mut address = network.get_tcp_address(addr);
        let listener = try!(address.listen());
        let reaper = Box::new(Reaper);
        accept_loop(listener, TaskSet::new(reaper)).wait(wait_scope, &mut event_port)
    }).expect("top level");

}