use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;

enum InternalMessage {
    NewClient(u64, mpsc::SyncSender<Vec<u8>>),
    ClientDisconnected(u64),
    NewMessage(Vec<u8>),
}

fn handle_client_read(mut stream: TcpStream, sender: mpsc::Sender<InternalMessage>) -> ::std::io::Result<()> {
    use std::io::Read;
    loop {
        let mut buf = [0];
        try!(stream.read_exact(&mut buf[..]));
        let mut buf_body = vec![0; buf[0] as usize];
        try!(stream.read_exact(&mut buf_body[..]));
        sender.send(InternalMessage::NewMessage(buf_body));
    }
}

fn handle_client_write(mut stream: TcpStream, rx: mpsc::Receiver<Vec<u8>>) -> ::std::io::Result<()> {
    use std::io::Write;
    loop {
        match rx.recv() {
            Ok(m) => {
                try!(stream.write_all(&[m.len() as u8]));
                try!(stream.write_all(&m[..]));
            }
            Err(_) => {
                // assume disconnected
                break
            }
        }
    }
    Ok(())
}

fn run() -> Result<(), ::std::io::Error> {

    let args: Vec<String> = ::std::env::args().collect();
    if args.len() != 2 {
        println!("usage: {} HOST:PORT", args[0]);
        return Ok(());
    }

    let addr_str = &args[1];
    let addr = addr_str.parse::<::std::net::SocketAddr>().unwrap();
    let listener = try!(TcpListener::bind(addr));

    let (tx, rx) = mpsc::channel();
    // start "global state" thread.
    ::std::thread::spawn(move || {
        let mut clients =
            ::std::collections::HashMap::<u64, mpsc::SyncSender<Vec<u8>>>::new();
        loop {
            match rx.recv().expect("receive error") {
                InternalMessage::NewClient(idx, s) => {
                    clients.insert(idx, s);
                }
                InternalMessage::ClientDisconnected(idx) => {
                    clients.remove(&idx);
                }
                InternalMessage::NewMessage(v) => {
                    for (idx, tx) in &clients {
                        tx.send(v.clone());
                    }
                }
            }
        }
    });

    println!("listening on {}", addr_str);
    // accept connections and process them, spawning a new thread for each one
    for stream in listener.incoming() {
        let tx = tx.clone();
        let mut count: u64 = 0;
        match stream {
            Ok(stream) => {
                let idx = count;
                count += 1;

                let (message_tx, message_rx) = mpsc::sync_channel(5);

                tx.send(InternalMessage::NewClient(idx, message_tx));

                let read_stream = try!(stream.try_clone());

                // start write thread
                ::std::thread::spawn(move|| {
                    handle_client_write(stream, message_rx)
                });

                // start read thread
                ::std::thread::spawn(move|| {
                    handle_client_read(read_stream, tx);
                });

            }
            Err(e) => { /* connection failed */ }
        }
    }

    Ok(())
}

pub fn main() {
    run().expect("top level error")
}
