# Zillions

A toy chat server spec and some sample implementations.

To help you choose among the many ways to write code to perform concurrent I/O.

## specification

The server is an executable with a single command-line argument,
which is an IP address to listen on.
The server runs forever, accepting and handling connections arriving on that address.

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


