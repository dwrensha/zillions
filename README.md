# Zillions

A toy chat server spec and some sample implementations,
to help you choose among the many ways to write code to perform concurrent I/O.

## contributing

Implementations live in the [impls](https://github.com/dwrensha/zillions/tree/master/impls)
directory. Please submit a pull request to add your own!

## specification

The server is an executable with a single command-line argument indicating the IP address to listen on.
The server runs forever, accepting and handling connections arriving on that address.

When the server is ready to receive connections, it prints a single line to stdout, of the form
"listening on [address]".

The purpose of the server is to forward messages to all connected clients.
There can be any number of connected clients active at a time.
Each message received from any client should be sent to all clients, including the sender.

A message is a sequence of bytes consisting of a 1-byte header
and a variable-length body. The body's length is equal to the value of the header byte
interpreted as an unsigned 8-bit integer.

Clients must receive messages in an order that is sequentially consistent with
the order that they were sent.

The server is allowed to drop messages -- i.e. to not send them to some senders.
Doing so may be necessary to prevent unbounded buffering in the case where the
server receives messages faster than it can send them out.
However, dropping too many messages may cause the server to receive lower scores in benchmark tests.

## testing

```
$ cargo build --release
$ cd impls/gjio && cargo build --release && cd ../..
$ ./target/release/stresstest -a 127.0.0.1:55555 ./impls/gjio/target/release/server
```

```
cargo run --release --bin commandline-client
```
