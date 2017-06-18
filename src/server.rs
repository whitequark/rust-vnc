use std::io::Write;
use std::net::{TcpStream, Shutdown};
use ::{protocol, Result};
use protocol::Message;

/// Definitions of events received by server from client.
#[derive(Debug)]
pub enum Event {
    /// A `SetPixelFormat` message sets the format in which pixel values should be sent in
    /// `FramebufferUpdate` messages. If the client does not send a `SetPixelFormat` message, then
    /// the server sends pixel values in its natural format as specified in the ServerInit message.
    SetPixelFormat(protocol::PixelFormat),

    /// A `SetEncodings` message sets the encoding types in which pixel data can be sent by the
    /// server. The order of the encoding types given in this message is a hint by the client as to
    /// its preference (the first encoding specified being most preferred). The server may or may
    /// not choose to make use of this hint. Pixel data may always be sent in raw encoding even if
    /// not specified explicitly here.
    ///
    /// In addition to genuine encodings, a client can request "pseudo-encodings" to declare to the
    /// server that it supports certain extensions to the protocol. A server that does not support
    /// the extension will simply ignore the pseudo-encoding. Note that this means the client must
    /// assume that the server does not support the extension until it gets some extension-specific
    /// confirmation from the server.
    SetEncodings(Vec<protocol::Encoding>),

    /// A `FramebufferUpdateRequest` message notifies the server that the client is interested in
    /// the area of the framebuffer specified by `x_position`, `y_position`, `width` and `height`.
    /// The server usually responds to a `FramebufferUpdateRequest` by sending a
    /// `FramebufferUpdate`. A single `FramebufferUpdate` may be sent in reply to several
    /// `FramebufferUpdateRequests`.
    ///
    /// The server assumes that the client keeps a copy of all parts of the framebuffer in which it
    /// is interested. This means that normally the server only needs to send incremental updates to
    /// the client.
    ///
    /// If the client has lost the contents of a particular area that it needs, then the client
    /// sends a FramebufferUpdateRequest with incremental set to false. This requests that the
    /// server send the entire contents of the specified area as soon as possible. The area will not
    /// be updated using the `CopyRect` encoding.
    ///
    /// If the client has not lost any contents of the area in which it is interested, then it sends
    /// a `FramebufferUpdateRequest` with incremental set to `true`. If and when there are changes
    /// to the specified area of the framebuffer, the server will send a `FramebufferUpdate`. Note
    /// that there may be an indefinite period between the `FramebufferUpdateRequest` and the
    /// `FramebufferUpdate`.
    FramebufferUpdateRequest {
        incremental: bool,
        x_position: u16,
        y_position: u16,
        width: u16,
        height: u16,
    },

    /// A `KeyEvent` message indicates a key press or release. `down` flag is `true` if the key is
    /// now pressed, and false if it is now released. The key itself is specified using the "keysym"
    /// values defined by the X Window System, even if the client or server is not running the X
    /// Window System.
    KeyEvent {
        down: bool,
        key: u32,
    },

    /// A `PointerEvent` message indicates either pointer movement or a pointer button press or
    /// release. The pointer is now at (`x_position`, `y_position`), and the current state of
    /// buttons 1 to 8 are represented by bits 0 to 7 of button-mask, respectively; 0 means up, 1
    /// means down (pressed).
    ///
    /// On a conventional mouse, buttons 1, 2, and 3 correspond to the left, middle, and right
    /// buttons on the mouse. On a wheel mouse, each step of the wheel upwards is represented by a
    /// press and release of button 4, and each step downwards is represented by a press and release
    /// of button 5.
    PointerEvent {
        button_mask: u8,
        x_position: u16,
        y_position: u16
    },

    /// RFB provides limited support for synchronizing the "cut buffer" of selected text between
    /// client and server. This message tells the server that the client has new ISO 8859-1
    /// (Latin-1) text in its cut buffer. Ends of lines are represented by the newline character
    /// (hex 0a) alone. No carriage-return (hex 0d) is used. There is no way to transfer text
    /// outside the Latin-1 character set.
    CutText(String),

    /// This encoding allows the client to send an extended key event containing a keycode, in
    /// addition to a keysym. The advantage of providing the keycode is that it enables the server
    /// to interpret the key event independantly of the clients’ locale specific keymap. This can
    /// be important for virtual desktops whose key input device requires scancodes, for example,
    /// virtual machines emulating a PS/2 keycode. Prior to this extension, RFB servers for such
    /// virtualization software would have to be configured with a keymap matching the client. With
    /// this extension it is sufficient for the guest operating system to be configured with the
    /// matching keymap. The VNC server is keymap independant.
    ///
    /// The `keysym` and `down`-flag fields also take the same values as described for the KeyEvent
    /// message. The keycode is the XT keycode that produced the keysym.
    ExtendedKeyEvent {
        down: bool,
        keysym: u32,
        keycode: u32,
    },
}

/// This structure provides basic server-side functionality of RDP protocol.
pub struct Server {
    stream: TcpStream,
}

impl Server {
    /// Constructs new `Server`.
    ///
    /// Returns new `Server` instance and `shared` flag.
    ///
    /// `shared` flag is `true` if the server should try to share the desktop by leaving other
    /// clients connected, and `false` if it should give exclusive access to this client by
    /// disconnecting all other clients.
    pub fn from_tcp_stream(mut stream: TcpStream,
                           width: u16,
                           height: u16,
                           pixel_format: protocol::PixelFormat,
                           name: String)
                           -> Result<(Server, bool)> {
        // Start version handshake - send highest supported version. Client may respond with lower
        // version but never higher.
        try!(protocol::Version::Rfb38.write_to(&mut stream));
        let version = try!(protocol::Version::read_from(&mut stream));

        // Start security handshake.
        // TODO: Add support for more security types and handle errors if negotiations fail.
        match version {
            protocol::Version::Rfb33 => {
                try!(protocol::SecurityType::None.write_to(&mut stream));
            }
            _ => {
                let security_types = vec![protocol::SecurityType::None];
                try!(protocol::SecurityTypes(security_types).write_to(&mut stream));
            }
        }

        let _security_type = try!(protocol::SecurityType::read_from(&mut stream));
        try!(protocol::SecurityResult::Succeeded.write_to(&mut stream));

        // Wait for client init message
        let client_init = try!(protocol::ClientInit::read_from(&mut stream));

        // Send server init message
        let server_init = protocol::ServerInit {
            framebuffer_width: width,
            framebuffer_height: height,
            pixel_format: pixel_format,
            name: name,
        };

        try!(server_init.write_to(&mut stream));

        Ok((Server { stream: stream }, client_init.shared))
    }

    /// Reads the socket and returns received event.
    pub fn read_event(&mut self) -> Result<Event> {
        match protocol::C2S::read_from(&mut self.stream) {
            Ok(package) => {
                match package {
                    protocol::C2S::SetPixelFormat(pixel_format) => {
                        Ok(Event::SetPixelFormat(pixel_format))
                    }
                    protocol::C2S::SetEncodings(encodings) => {
                        Ok(Event::SetEncodings(encodings))
                    }
                    protocol::C2S::FramebufferUpdateRequest {
                        incremental,
                        x_position,
                        y_position,
                        width,
                        height,
                    } => {
                        Ok(Event::FramebufferUpdateRequest {
                            incremental,
                            x_position,
                            y_position,
                            width,
                            height,
                        })
                    }
                    protocol::C2S::KeyEvent { down, key } => {
                        Ok(Event::KeyEvent { down, key })
                    }
                    protocol::C2S::PointerEvent { button_mask, x_position, y_position } => {
                        Ok(Event::PointerEvent { button_mask, x_position, y_position })
                    }
                    protocol::C2S::CutText(clipboard) => {
                        Ok(Event::CutText(clipboard))
                    }
                    protocol::C2S::ExtendedKeyEvent { down, keysym, keycode } => {
                        Ok(Event::ExtendedKeyEvent { down, keysym, keycode })
                    }
                }
            }
            Err(error) => Err(error)
        }
    }

    /// Sends header of `FramebufferUpdate` message containing number of rectangles to be sent.
    ///
    /// Call to this method must be followed by `count` calls to `send_rectangle_header`.
    pub fn send_framebuffer_update_header(&mut self, count: u16) -> Result<()> {
        try!(protocol::S2C::FramebufferUpdate{count}.write_to(&mut self.stream));
        Ok(())
    }

    /// Sends rectangle header.
    ///
    /// The rectangle header must be followed by the pixel data in the specified encoding.
    pub fn send_rectangle_header(&mut self,
                                 x: u16,
                                 y: u16,
                                 width: u16,
                                 height: u16,
                                 encoding: protocol::Encoding)
                                 -> Result<()> {
        try!(protocol::Rectangle {
            x_position: x,
            y_position: y,
            width: width,
            height: height,
            encoding: encoding,
        }.write_to(&mut self.stream));
        Ok(())
    }

    /// Writes raw data to the socket.
    ///
    /// This method may be used to send pixel data following rectangle header sent by
    /// `send_rectangle_header` method.
    pub fn send_raw_data(&mut self, data: &[u8]) -> Result<()> {
        try!(self.stream.write_all(data));
        Ok(())
    }

    /// Shuts down communication over TCP stream in both directions.
    pub fn disconnect(self) -> Result<()> {
        try!(self.stream.shutdown(Shutdown::Both));
        Ok(())
    }
}
