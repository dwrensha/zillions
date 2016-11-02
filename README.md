# Zillions

A toy chat server spec and some sample implementations,
to help you choose among the many ways to write code to perform concurrent I/O.

## specification

The server is an executable with a single command-line argument indicating an IP address on which to listen.
The server runs forever, accepting and handling TCP connections arriving on that address.

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

## contributing

Implementations live in the [impls](https://github.com/dwrensha/zillions/tree/master/impls)
directory. Please submit a pull request to add your own!

## testing

This repo includes some tools to allow you to test implementations and to compare
their performance.

```
$ cargo build --release
$ cd impls/gjio && cargo build --release && cd ../..
$ ./target/release/stresstest -a 127.0.0.1:55555 ./impls/gjio/target/release/server
```

```
cargo run --release --bin commandline-client
```

## wish list

Our goal is to collect many different implementations of the server spec to
illustrate a wide variety of concurrency styles. Below are listed some
libraries that might be particularly interesting to try.

- Rust
  * raw [mio](https://github.com/carllerche/mio)
  * [amy](https://github.com/andrewjstone/amy)
  * [libfringe](https://github.com/nathan7/libfringe)
  * [mioco](https://github.com/dpc/mioco)
- other languages
  * [curio](https://github.com/dabeaz/curio)
  * C\# [async/await](https://msdn.microsoft.com/en-us/library/mt674882.aspx)
  * Goroutines
  * [libmill](http://libmill.org/)
  * Haskell