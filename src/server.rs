use std::io::Write;
use std::net::{TcpStream, Shutdown};
use byteorder::{BigEndian, WriteBytesExt};
use ::{protocol, Rect, Result};
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
        rect: Rect,
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
    /// to interpret the key event independently of the clients’ locale specific keymap. This can
    /// be important for virtual desktops whose key input device requires scancodes, for example,
    /// virtual machines emulating a PS/2 keycode. Prior to this extension, RFB servers for such
    /// virtualization software would have to be configured with a keymap matching the client. With
    /// this extension it is sufficient for the guest operating system to be configured with the
    /// matching keymap. The VNC server is keymap independent.
    ///
    /// The `keysym` and `down`-flag fields also take the same values as described for the KeyEvent
    /// message. The keycode is the XT keycode that produced the keysym.
    ExtendedKeyEvent {
        down: bool,
        keysym: u32,
        keycode: u32,
    },
}

/// Helper data structure containing data to be sent by server in messages containing rectangles.
#[derive(Debug)]
enum Update<'a> {
    Raw {
        rect: Rect,
        pixel_data: &'a [u8],
    },
    CopyRect {
        dst: Rect,
        src_x_position: u16,
        src_y_position: u16,
    },
    Zrle {
        rect: Rect,
        zlib_data: &'a [u8],
    },
    SetCursor {
        size: (u16, u16),
        hotspot: (u16, u16),
        pixels: &'a [u8],
        mask_bits: &'a [u8],
    },
    DesktopSize {
        width: u16,
        height: u16,
    },
    Encoding { encoding: protocol::Encoding },
}

impl<'a> Update<'a> {
    /// Checks validity of given `Update`. Panics if it is not valid.
    fn check(&self, validation_data: &ValidationData) {
        match *self {
            Update::Raw { ref rect, pixel_data } => {
                let expected_num_bytes = rect.width as usize *
                                         rect.height as usize *
                                         validation_data.bytes_per_pixel as usize;
                if expected_num_bytes != pixel_data.len() {
                    panic!("Expected data length for rectangle {:?} is {} while given {}",
                           rect,
                           expected_num_bytes,
                           pixel_data.len());
                }
            }
            Update::CopyRect { dst: _, src_x_position: _, src_y_position: _ } => {
                // No check is needed
            }
            Update::Zrle { rect: _, zlib_data } => {
                if zlib_data.len() > u32::max_value() as usize {
                    panic!("Maximal length of compressed data is {}", u32::max_value());
                }
            }
            Update::SetCursor { size: (width, height), hotspot: _, pixels, mask_bits } => {
                // Check pixel data length
                let expected_num_bytes = width as usize *
                                         height as usize *
                                         validation_data.bytes_per_pixel as usize;
                if expected_num_bytes != pixels.len() {
                    panic!("Expected data length is {} while given {}",
                           expected_num_bytes,
                           pixels.len());
                }

                // Check bit mask length
                let expected_num_bytes = ((width as usize + 7) / 8) * height as usize;
                if expected_num_bytes != mask_bits.len() {
                    panic!("Expected bit mask length is {} while given {}",
                           expected_num_bytes,
                           mask_bits.len());
                }
            }
            Update::DesktopSize { width: _, height: _ } => {
                // No check is needed
            }
            Update::Encoding { encoding: _ } => {
                // No check is needed
            }
        }
    }

    /// Serializes `Update` to given stream.
    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        match *self {
            Update::Raw { ref rect, pixel_data } => {
                try!(rect.write_to(writer));
                try!(protocol::Encoding::Raw.write_to(writer));
                try!(writer.write_all(pixel_data));
            }
            Update::CopyRect { ref dst, src_x_position, src_y_position } => {
                try!(dst.write_to(writer));
                try!(protocol::Encoding::CopyRect.write_to(writer));
                try!(writer.write_u16::<BigEndian>(src_x_position));
                try!(writer.write_u16::<BigEndian>(src_y_position));
            }
            Update::Zrle { ref rect, zlib_data } => {
                try!(rect.write_to(writer));
                try!(protocol::Encoding::Zrle.write_to(writer));
                try!(writer.write_u32::<BigEndian>(zlib_data.len() as u32));
                try!(writer.write_all(zlib_data));
            }
            Update::SetCursor { size, hotspot, pixels, mask_bits } => {
                try!(writer.write_u16::<BigEndian>(hotspot.0));
                try!(writer.write_u16::<BigEndian>(hotspot.1));
                try!(writer.write_u16::<BigEndian>(size.0));
                try!(writer.write_u16::<BigEndian>(size.1));
                try!(protocol::Encoding::Cursor.write_to(writer));
                try!(writer.write_all(pixels));
                try!(writer.write_all(mask_bits));
            }
            Update::DesktopSize { width, height } => {
                try!(writer.write_u16::<BigEndian>(0));
                try!(writer.write_u16::<BigEndian>(0));
                try!(writer.write_u16::<BigEndian>(width));
                try!(writer.write_u16::<BigEndian>(height));
                try!(protocol::Encoding::DesktopSize.write_to(writer));
            }
            Update::Encoding { encoding } => {
                try!(Rect::new_empty().write_to(writer));
                try!(encoding.write_to(writer));
            }
        }
        Ok(())
    }
}

/// Builder of `FramebufferUpdate` message.
pub struct FramebufferUpdate<'a> {
    updates: Vec<Update<'a>>,
}

impl<'a> FramebufferUpdate<'a> {
    /// Constructs new `FramebufferUpdate`.
    pub fn new() -> Self {
        FramebufferUpdate {
            updates: Vec::new(),
        }
    }

    /// Adds raw pixel data.
    pub fn add_raw_pixels(&mut self, rect: Rect, pixel_data: &'a [u8]) -> &mut Self {
        let update = Update::Raw {
            rect: rect,
            pixel_data: pixel_data
        };

        self.updates.push(update);
        self
    }

    /// Adds `CopyRect` update message instructing client to reuse pixel data it already owns.
    pub fn add_copy_rect(&mut self,
                         dst: Rect,
                         src_x_position: u16,
                         src_y_position: u16)
                         -> &mut Self {
        let update = Update::CopyRect {
            dst: dst,
            src_x_position: src_x_position,
            src_y_position: src_y_position,
        };

        self.updates.push(update);
        self
    }

    /// Adds compressed pixel data.
    ///
    /// TODO: add method taking uncompressed data and compressing them.
    pub fn add_compressed_pixels(&mut self, rect: Rect, zlib_data: &'a [u8]) -> &mut Self {
        let update = Update::Zrle {
            rect: rect,
            zlib_data: zlib_data
        };

        self.updates.push(update);
        self
    }

    /// Add data for drawing cursor.
    pub fn add_cursor(&mut self,
                      width: u16,
                      height: u16,
                      hotspot_x: u16,
                      hotspot_y: u16,
                      pixels: &'a [u8],
                      mask_bits: &'a [u8])
                      -> &mut Self {
        let update = Update::SetCursor {
            size: (width, height),
            hotspot: (hotspot_x, hotspot_y),
            pixels: pixels,
            mask_bits: mask_bits
        };

        self.updates.push(update);
        self
    }

    /// Adds notification about framebuffer resize.
    pub fn add_desktop_size(&mut self, width: u16, height: u16) -> &mut Self {
        let update = Update::DesktopSize {
            width: width,
            height: height,
        };

        self.updates.push(update);
        self
    }

    /// Adds confirmation of support of pseudo-encoding.
    pub fn add_pseudo_encoding(&mut self, encoding: protocol::Encoding) -> &mut Self {
        let update = Update::Encoding { encoding: encoding };

        self.updates.push(update);
        self
    }

    /// Checks if all updates are valid.
    ///
    /// Panics if any of the updates is not valid.
    fn check(&self, validation_data: &ValidationData) {
        for update in self.updates.iter() {
            update.check(validation_data);
        }
    }

    /// Serializes this structure and sends it using given `writer`.
    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        for chunk in self.updates.chunks(u16::max_value() as usize) {
            let count = chunk.len() as u16;
             try!(protocol::S2C::FramebufferUpdate{count}.write_to(writer));
             for update in chunk {
                try!(update.write_to(writer));
             }
        }
        Ok(())
    }
}

/// Gathers all data needed to validate framebuffer updates.
struct ValidationData {
    /// Number of bytes per pixel used to check validity of sent data extracted from `PixelFormat`.
    bytes_per_pixel: u16,
}

impl ValidationData {
    /// Constructs new `ValidationData`.
    fn new(pixel_format: &protocol::PixelFormat) -> Self {
        let mut mine = ValidationData { bytes_per_pixel: 0 };
        mine.update(pixel_format);
        mine
    }

    /// Updates bytes per pixel from `PixelFormat`.
    fn update(&mut self, pixel_format: &protocol::PixelFormat) {
        self.bytes_per_pixel = (pixel_format.bits_per_pixel as u16 + 7) / 8;
    }
}

/// This structure provides basic server-side functionality of RDP protocol.
pub struct Server {
    stream: TcpStream,
    validation_data: ValidationData
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

        Ok((Server {
            stream: stream,
            validation_data: ValidationData::new(&pixel_format),
        }, client_init.shared))
    }

    /// Reads the socket and returns received event.
    pub fn read_event(&mut self) -> Result<Event> {
        match protocol::C2S::read_from(&mut self.stream) {
            Ok(package) => {
                match package {
                    protocol::C2S::SetPixelFormat(pixel_format) => {
                        // Update bytes per pixel number. Server must obey this message and from
                        // now send data in format requested by client.
                        self.validation_data.update(&pixel_format);
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
                            rect: Rect::new(x_position, y_position, width, height),
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

    /// Sends `FramebufferUpdate` message.
    ///
    /// Panics if given updates are not valid. All validity checks are done before sending any
    /// update.
    pub fn send_update(&mut self, updates: &FramebufferUpdate) -> Result<()> {
        updates.check(&self.validation_data);
        try!(updates.write_to(&mut self.stream));
        Ok(())
    }

    /// Shuts down communication over TCP stream in both directions.
    pub fn disconnect(self) -> Result<()> {
        try!(self.stream.shutdown(Shutdown::Both));
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::{protocol, Rect, Update, ValidationData};

    /// Checks if `ValidationData` correctly converts bits per pixel from `PixelFormat` to bytes
    /// per pixel.
    #[test]
    fn check_if_validation_data_correctly_rounds_bits_to_bytes() {
        let mut format = protocol::PixelFormat::new_rgb8888();
        let test_data = vec![(8, 1), (23, 3), (24, 3), (25, 4), (31, 4), (32, 4), (33, 5)];
        for (bits, expected_bytes) in test_data {
            format.bits_per_pixel = bits;
            let data = ValidationData::new(&format);
            assert_eq!(data.bytes_per_pixel, expected_bytes);
        }
    }

    /// Checks if `Update::Raw` accepts valid data both in case of very small and big buffer which
    /// could cause `u16` overflow.
    #[test]
    fn check_if_raw_update_accepts_valid_data() {
        let data = vec![0; 4 * 800 * 100];
        let pixel_format = protocol::PixelFormat::new_rgb8888();
        let validation_data = ValidationData::new(&pixel_format);

        // Small rectangle
        Update::Raw {
            rect: Rect::new(0, 0, 8, 8),
            pixel_data: &data[0 .. (4 * 8 * 8)],
        }.check(&validation_data);

        // Big rectangle (bigger than `u16::MAX`)
        Update::Raw {
            rect: Rect::new(0, 0, 800, 100),
            pixel_data: &data,
        }.check(&validation_data);
    }

    /// Checks if `Update::Raw` rejects data with invalid length.
    #[test]
    #[should_panic]
    fn check_if_raw_update_rejects_invalid_data() {
        let data = vec![0; 5];
        let pixel_format = protocol::PixelFormat::new_rgb8888();
        let validation_data = ValidationData::new(&pixel_format);

        Update::Raw {
            rect: Rect::new(0, 0, 8, 8),
            pixel_data: &data,
        }.check(&validation_data);
    }

    /// Checks if `Update::SetCursor` accepts valid data both in case of very small and big buffer
    /// which could cause `u16` overflow.
    #[test]
    fn check_if_set_cursor_update_accepts_valid_data() {
        let data = vec![0; 4 * 800 * 100];
        let pixel_format = protocol::PixelFormat::new_rgb8888();
        let validation_data = ValidationData::new(&pixel_format);

        // Small rectangle
        Update::SetCursor {
            size: (8, 8),
            hotspot: (0, 0),
            pixels: &data[0 .. (4 * 8 * 8)],
            mask_bits: &data[0 .. 8],
        }.check(&validation_data);

        // Big rectangle (bigger than `u16::MAX`)
        Update::SetCursor {
            size: (800, 100),
            hotspot: (0, 0),
            pixels: &data,
            mask_bits: &data[0 .. 10000],
        }.check(&validation_data);
    }

    /// Checks if `Update::SetCursor` rejects data with invalid pixel length.
    #[test]
    #[should_panic]
    fn check_if_set_cursor_update_rejects_invalid_pixel_data() {
        let data = vec![0; 15];
        let pixel_format = protocol::PixelFormat::new_rgb8888();
        let validation_data = ValidationData::new(&pixel_format);

        Update::SetCursor {
            size: (8, 8),
            hotspot: (0, 0),
            pixels: &data,
            mask_bits: &data[0 .. 8],
        }.check(&validation_data);
    }

    /// Checks if `Update::SetCursor` rejects data with invalid bit mask length.
    #[test]
    #[should_panic]
    fn check_if_set_cursor_update_rejects_invalid_bit_mask_data() {
        let data = vec![0; 16];
        let pixel_format = protocol::PixelFormat::new_rgb8888();
        let validation_data = ValidationData::new(&pixel_format);

        Update::SetCursor {
            size: (8, 8),
            hotspot: (0, 0),
            pixels: &data,
            mask_bits: &data[0 .. 7],
        }.check(&validation_data);
    }
}
