# Zillions

A toy server spec and some sample implementations.

To help you choose among the many ways to write code to perform concurrent I/O.

## specification

The server is an executable with a single command-line argument,
which is an IP address to listen on.
The server runs an infinite loop where it accepts and handles
connections arriving on that address.
Connections have two types, distinguished by the first incoming byte received on them.
A *publisher* connection sends 0 for its first byte.
A *subscriber* connection sends 1 for its first byte.

The purpose of the server is to receive messages from publishers
and to forward them to subscribers. There can be any number of publishers
and subscribers active at a given time.
Each message received from any publisher should be sent to all subscribers.

A message is a sequence of bytes consisting of a 1-byte header
and a variable-length body. The body's length is equal to the value of the header byte
interpreted as an unsigned 8-bit integer.

The server must keep track of how many messages it has received from each publisher.
When a publisher's incoming stream reaches EOF, the server must attempt to
send back to the publish the number of messages received from that publisher, as an
unsigned 64-bit integer encoded in little-endian byte order.
After that, the server must close the connection.

Subscribers must receive messages in an order that is sequentially consistent with
the order that publishers sent them. The server is allowed to drop messages -- i.e.
to not send them to some subscribers. However, doing so will cause the server to
receive lower scores in benchmark tests.

