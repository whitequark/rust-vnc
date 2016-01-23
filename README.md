rust-vnc
========

rust-vnc is a library implementing the VNC protocol and the client
state machine. The crate also provides two binaries:

  * A fully functional VNC client based on SDL2;
  * A buffering VNC proxy.

How?
----

Entirely by mistake.

I've encountered a very strange failure, wherein a Xen domU with a VNC console
attached to it would not send any framebuffer updates. By the time I've
finally realized that my VNC clients were fine and something was wrong with
Xen, it was too late: rust-vnc was practically complete. Then I spent twice
as much time perfecting the handling of keyboard layouts for some reason.

Where?
------

To launch the VNC client or proxy, run `cargo install vnc` and then
`rvncclient --help` or `rvncproxy --help`.

To use the VNC library in your project, add the following to `Cargo.toml`:

```toml
[dependencies]
vnc = "^0.1"
```

Why?
----

The vnc crate implements serialization and deserialization for all of
the [core VNC protocol][vnc], and a largely complete client state machine.

The rvncclient tool is a quite usable VNC client, as it implements
several extensions that cut down unnecessary data transfers; as a bonus
it can be used for education and troubleshooting, as it will output
a human-readable dump of the VNC messages if ran with `RUST_LOG` environment
variable set to `debug`.

The rvncproxy tool is a proxy that sits in the middle of a VNC connection
and buffers all server-to-client packets so that the server would (almost)
never block, even the last mile to the client is very slow and/or
has high latency. The proxy also supports `RUST_LOG=debug` setting.
Note that the proxy will strip (and warn about) authentication methods and
encodings it does not understand, since it is not possible to decode
VNC framing otherwise.

[vnc]: https://www.realvnc.com/docs/rfbproto.pdf

Why not?
--------

I didn't really intend to write this library at all, and as such it has
some drawbacks:

  * No server state machine or compression.
  * No encryption or authentication.
  * No pixel encoding support beyond Raw and CopyRect.
  * No inline documentation (but the [signatures][doc] and the [client][]
    could be helpful already).

That said, the library was written with the full VNC protocol in mind,
and it should be straightforward to extend the library to support
any of the above, should a need arise.

The rvncproxy tool was written with a specific misbehaving server
implementation in mind, but then it turned out that server misbehaved
in a completely different way, so it's not really useful for anything.

[doc]: https://whitequark.github.io/rust-vnc/vnc/
[client]: src/bin/rvncclient.rs

Whereto?
--------

rust-vnc is distributed under the terms of both the MIT license
and the Apache License (Version 2.0).

See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT)
for details.
