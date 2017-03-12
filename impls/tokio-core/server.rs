extern crate futures;
extern crate tokio_core;

use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::io;
use std::iter;
use std::net::SocketAddr;
use std::rc::Rc;

use futures::Future;
use futures::stream::{self, Stream};
use futures::sync::mpsc;
use tokio_core::io::{read_exact, write_all, Io};
use tokio_core::net::{TcpListener, TcpStream};
use tokio_core::reactor::Core;

const MAX_BUFFERED: usize = 5;

fn main() {
    // Parse out what address we're going to listen on, defaulting to "something
    // reasonable"
    let addr = env::args().skip(1).next();
    let addr = addr.unwrap_or("127.0.0.1:8080".to_string());
    let addr: SocketAddr = addr.parse().unwrap();

    // Create the global state of our program. This involves the event loop (a
    // `Core`) to run all our I/O on, a TCP listener which we're going to accept
    // connections form, and finally the global hash map we're storing all
    // connection state in.
    let mut core = Core::new().unwrap();
    let handle = core.handle();
    let srv = TcpListener::bind(&addr, &handle).unwrap();
    println!("listening on {}", addr);
    let map = Rc::new(RefCell::new(HashMap::new()));

    // Create a small loop which will spawn off a new task for all accepted
    // connections. Each task will handle the messages for that connection
    // specifically, as well as listening for messages from other connections.
    //
    // Note that `futures::lazy` here is used for the call to `Io::split` below
    // in the `client` function, but it can mostly just be ignored.
    let srv = srv.incoming().for_each(|(socket, addr)| {
        let map = map.clone();
        handle.spawn(futures::lazy(move || {
            client(socket, addr, map)
        }));
        Ok(())
    });

    core.run(srv).unwrap();
}

/// Internal state saved for each client.
///
/// The "global hash map" is a hash of socket addresses to instances of this
/// type. An instance is available for all connected clients.
struct Client {
    // A channel to send messages over to be received and written out.
    tx: mpsc::UnboundedSender<Vec<u8>>,

    // Current number of pending messages. If too many messages are pending then
    // messages to this client are dropped on the floor.
    size: usize,
}

fn client(socket: TcpStream,
          addr: SocketAddr,
          map: Rc<RefCell<HashMap<SocketAddr, Client>>>)
          -> Box<Future<Item=(), Error=()>> {
    // First up, register our new client by creating an entry in the global
    // state map.
    let (tx, rx) = mpsc::unbounded();
    assert!(map.borrow_mut().insert(addr, Client {
        tx: tx,
        size: 0,
    }).is_none());

    // Next, we call the `Io::split` function here to easily work with both
    // halves of the socket as an instance of `Read` and `Write`. This'll allow
    // us to independently define the logic below without having to worry about
    // Rust's ownership of the original value.
    let (reader, writer) = socket.split();

    // Here we define how to read the socket. A small infinite iterator is
    // created to form a loop over reading messages off the socket. We use the
    // `read_exact` combinator to read messages from the socket, and once a
    // message is read it's dispatched to all other clients.
    //
    // Note that once a message is read we finish one iteration of the `fold`
    // and the reader is then passed on to the next to execute.
    //
    // Also note that this is where the backpressure logic for connected clients
    // comes into play. We don't send messages to clients that have too many
    // messages queued up already.
    let map2 = map.clone();
    let infinite = stream::iter(iter::repeat(()).map(Ok::<(), io::Error>));
    let reader = infinite.fold(reader, move |reader, ()| {
        let size = read_exact(reader, [0u8]);

        let msg = size.and_then(|(rd, size)| {
            let buf = vec![0u8; size[0] as usize];
            read_exact(rd, buf)
        });

        let map = map2.clone();
        msg.map(move |(rd, buf)| {
            for peer in map.borrow_mut().values_mut() {
                if peer.size < MAX_BUFFERED {
                    peer.size += 1;
                    peer.tx.send(buf.clone()).unwrap();
                }
            }
            rd
        })
    });

    // Here we define how to write messages to our client. This simply involves
    // iterating over all messages coming off the channel and writing them out
    // as per our protocol.
    let map2 = map.clone();
    let rx1 = rx.map_err(|_| -> ::std::io::Error { unreachable!() });
    let writer = rx1.fold(writer, move |writer, msg| {
        map2.borrow_mut().get_mut(&addr).unwrap().size -= 1;
        write_all(writer, [msg.len() as u8]).and_then(|(wr, _)| {
            write_all(wr, msg).map(|p| p.0)
        })
    });

    // Finally, we `select` over the read and the write halves. This means that
    // once either hits an error or finishes we're done. Note that the most
    // likely thing to happen here is the `reader` hits an error due to EOF (in
    // line with our use of `read_exact` above) which will cause the `select`
    // future to resolve.
    //
    // Once the reader/writer halves are done, we deregister ourselves from the
    // global map of connections. to free up our resources.
    let reader = reader.map(|_| ()).map_err(|_| ());
    let writer = writer.map(|_| ()).map_err(|_| ());
    Box::new(reader.select(writer).then(move |_| {
        assert!(map.borrow_mut().remove(&addr).is_some());
        Ok(())
    }))
}
