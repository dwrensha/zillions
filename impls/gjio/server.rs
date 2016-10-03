extern crate gj;
extern crate gjio;
extern crate byteorder;
extern crate slab;

use gj::{EventLoop, Promise, PromiseFulfiller, TaskReaper, TaskSet};

fn accept_loop(listener: gjio::SocketListener,
               mut task_set: TaskSet<(), ::std::io::Error>)
               -> Promise<(), ::std::io::Error>
{
    unimplemented!()
//    listener.accept().then(move |stream| {
//        accept_loop(listener, task_set)
//    })
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
