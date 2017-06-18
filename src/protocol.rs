use std::io::{ErrorKind as IoErrorKind, Read, Write};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use ::{Error, Result};

pub trait Message {
    fn read_from<R: Read>(reader: &mut R) -> Result<Self> where Self: Sized;
    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()>;
}

impl Message for Vec<u8> {
    fn read_from<R: Read>(reader: &mut R) -> Result<Vec<u8>> {
        let length = try!(reader.read_u32::<BigEndian>());
        let mut buffer = vec![0; length as usize];
        try!(reader.read_exact(&mut buffer));
        Ok(buffer)
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        let length = self.len() as u32; // TODO: check?
        try!(writer.write_u32::<BigEndian>(length));
        try!(writer.write_all(&self));
        Ok(())
    }
}

/* All strings in VNC are either ASCII or Latin-1, both of which
   are embedded in Unicode. */
impl Message for String {
    fn read_from<R: Read>(reader: &mut R) -> Result<String> {
        let length = try!(reader.read_u32::<BigEndian>());
        let mut string = vec![0; length as usize];
        try!(reader.read_exact(&mut string));
        Ok(string.iter().map(|c| *c as char).collect())
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        let length = self.len() as u32; // TODO: check?
        try!(writer.write_u32::<BigEndian>(length));
        try!(writer.write_all(&self.chars().map(|c| c as u8).collect::<Vec<u8>>()));
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Version {
    Rfb33,
    Rfb37,
    Rfb38
}

impl Message for Version {
    fn read_from<R: Read>(reader: &mut R) -> Result<Version> {
        let mut buf = [0; 12];
        try!(reader.read_exact(&mut buf));
        match &buf {
            b"RFB 003.003\n" => Ok(Version::Rfb33),
            b"RFB 003.007\n" => Ok(Version::Rfb37),
            b"RFB 003.008\n" => Ok(Version::Rfb38),
            // Apple remote desktop
            b"RFB 003.889\n" => Ok(Version::Rfb38),
            _ => Err(Error::Unexpected("protocol version"))
        }
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        try!(match self {
            &Version::Rfb33 => writer.write_all(b"RFB 003.003\n"),
            &Version::Rfb37 => writer.write_all(b"RFB 003.007\n"),
            &Version::Rfb38 => writer.write_all(b"RFB 003.008\n"),
        });
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityType {
    Unknown(u8),
    // core spec
    Invalid,
    None,
    VncAuthentication,
    // extensions
    AppleRemoteDesktop,
}

impl Message for SecurityType {
    fn read_from<R: Read>(reader: &mut R) -> Result<SecurityType> {
        let security_type = try!(reader.read_u8());
        match security_type {
            0  => Ok(SecurityType::Invalid),
            1  => Ok(SecurityType::None),
            2  => Ok(SecurityType::VncAuthentication),
            30 => Ok(SecurityType::AppleRemoteDesktop),
            n  => Ok(SecurityType::Unknown(n))
        }
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        let security_type = match self {
            &SecurityType::Invalid => 0,
            &SecurityType::None => 1,
            &SecurityType::VncAuthentication => 2,
            &SecurityType::AppleRemoteDesktop => 30,
            &SecurityType::Unknown(n) => n
        };
        try!(writer.write_u8(security_type));
        Ok(())
    }
}

#[derive(Debug)]
pub struct SecurityTypes(pub Vec<SecurityType>);

impl Message for SecurityTypes {
    fn read_from<R: Read>(reader: &mut R) -> Result<SecurityTypes> {
        let count = try!(reader.read_u8());
        let mut security_types = Vec::new();
        for _ in 0..count {
            security_types.push(try!(SecurityType::read_from(reader)))
        }
        Ok(SecurityTypes(security_types))
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        let count = self.0.len() as u8; // TODO: check?
        try!(writer.write_u8(count));
        for security_type in &self.0 {
            try!(security_type.write_to(writer));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityResult {
    Succeeded,
    Failed
}

impl Message for SecurityResult {
    fn read_from<R: Read>(reader: &mut R) -> Result<SecurityResult> {
        let result = try!(reader.read_u32::<BigEndian>());
        match result {
            0 => Ok(SecurityResult::Succeeded),
            1 => Ok(SecurityResult::Failed),
            _ => Err(Error::Unexpected("security result"))
        }
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        let result = match self {
            &SecurityResult::Succeeded => 0,
            &SecurityResult::Failed => 1
        };
        try!(writer.write_u32::<BigEndian>(result));
        Ok(())
    }
}

#[derive(Debug)]
pub struct AppleAuthHandshake {
    pub generator: u16,
    pub prime: Vec<u8>,
    pub peer_key: Vec<u8>,
}

impl Message for AppleAuthHandshake {
    fn read_from<R: Read>(reader: &mut R) -> Result<Self> {
        let generator = try!(reader.read_u16::<BigEndian>());
        let key_length = try!(reader.read_u16::<BigEndian>());

        let mut prime = vec![0; key_length as usize];
        try!(reader.read_exact(&mut prime));

        let mut peer_key = vec![0; key_length as usize];
        try!(reader.read_exact(&mut peer_key));

        Ok(AppleAuthHandshake {
            generator: generator,
            prime: prime,
            peer_key: peer_key,
        })
    }

    fn write_to<W: Write>(&self, _writer: &mut W) -> Result<()> {
        unreachable!()
    }
}

#[allow(dead_code)]
pub struct AppleAuthResponse {
    pub ciphertext: [u8; 128],
    pub pub_key: Vec<u8>,
}

impl Message for AppleAuthResponse {
    fn read_from<R: Read>(_reader: &mut R) -> Result<Self> {
        unreachable!()
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        try!(writer.write_all(&self.ciphertext));
        try!(writer.write_all(&self.pub_key));
        Ok(())
    }
}

#[derive(Debug)]
pub struct ClientInit {
    pub shared: bool
}

impl Message for ClientInit {
    fn read_from<R: Read>(reader: &mut R) -> Result<ClientInit> {
        Ok(ClientInit {
            shared: try!(reader.read_u8()) != 0
        })
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        try!(writer.write_u8(if self.shared { 1 } else { 0 }));
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelFormat {
    pub bits_per_pixel: u8,
    pub depth:          u8,
    pub big_endian:     bool,
    pub true_colour:    bool,
    pub red_max:        u16,
    pub green_max:      u16,
    pub blue_max:       u16,
    pub red_shift:      u8,
    pub green_shift:    u8,
    pub blue_shift:     u8,
}

impl PixelFormat {
    /// Creates RGB pixel format with 4 bytes per pixel and 3 bytes of depth.
    pub fn new_rgb8888() -> Self {
        PixelFormat {
            bits_per_pixel: 32,
            depth: 24,
            big_endian: true,
            true_colour: true,
            red_max: 255,
            green_max: 255,
            blue_max: 255,
            red_shift: 0,
            green_shift: 8,
            blue_shift: 16,
        }
    }

    /// Creates BGR pixel format with 4 bytes per pixel and 3 bytes of depth.
    pub fn new_bgr8888() -> Self {
        PixelFormat {
            bits_per_pixel: 32,
            depth: 24,
            big_endian: true,
            true_colour: true,
            red_max: 255,
            green_max: 255,
            blue_max: 255,
            red_shift: 16,
            green_shift: 8,
            blue_shift: 0,
        }
    }
}

impl Message for PixelFormat {
    fn read_from<R: Read>(reader: &mut R) -> Result<PixelFormat> {
        let pixel_format = PixelFormat {
            bits_per_pixel: try!(reader.read_u8()),
            depth:          try!(reader.read_u8()),
            big_endian:     try!(reader.read_u8()) != 0,
            true_colour:    try!(reader.read_u8()) != 0,
            red_max:        try!(reader.read_u16::<BigEndian>()),
            green_max:      try!(reader.read_u16::<BigEndian>()),
            blue_max:       try!(reader.read_u16::<BigEndian>()),
            red_shift:      try!(reader.read_u8()),
            green_shift:    try!(reader.read_u8()),
            blue_shift:     try!(reader.read_u8()),
        };
        try!(reader.read_exact(&mut [0u8; 3]));
        Ok(pixel_format)
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        try!(writer.write_u8(self.bits_per_pixel));
        try!(writer.write_u8(self.depth));
        try!(writer.write_u8(if self.big_endian { 1 } else { 0 }));
        try!(writer.write_u8(if self.true_colour { 1 } else { 0 }));
        try!(writer.write_u16::<BigEndian>(self.red_max));
        try!(writer.write_u16::<BigEndian>(self.green_max));
        try!(writer.write_u16::<BigEndian>(self.blue_max));
        try!(writer.write_u8(self.red_shift));
        try!(writer.write_u8(self.green_shift));
        try!(writer.write_u8(self.blue_shift));
        try!(writer.write_all(&[0u8; 3]));
        Ok(())
    }
}

#[derive(Debug)]
pub struct ServerInit {
    pub framebuffer_width:  u16,
    pub framebuffer_height: u16,
    pub pixel_format:       PixelFormat,
    pub name:               String
}

impl Message for ServerInit {
    fn read_from<R: Read>(reader: &mut R) -> Result<ServerInit> {
        Ok(ServerInit {
            framebuffer_width:  try!(reader.read_u16::<BigEndian>()),
            framebuffer_height: try!(reader.read_u16::<BigEndian>()),
            pixel_format:       try!(PixelFormat::read_from(reader)),
            name:               try!(String::read_from(reader))
        })
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        try!(writer.write_u16::<BigEndian>(self.framebuffer_width));
        try!(writer.write_u16::<BigEndian>(self.framebuffer_height));
        try!(PixelFormat::write_to(&self.pixel_format, writer));
        try!(String::write_to(&self.name, writer));
        Ok(())
    }
}

#[derive(Debug)]
pub struct CopyRect {
    pub src_x_position: u16,
    pub src_y_position: u16,
}

impl Message for CopyRect {
    fn read_from<R: Read>(reader: &mut R) -> Result<CopyRect> {
        Ok(CopyRect {
            src_x_position: try!(reader.read_u16::<BigEndian>()),
            src_y_position: try!(reader.read_u16::<BigEndian>())
        })
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        try!(writer.write_u16::<BigEndian>(self.src_x_position));
        try!(writer.write_u16::<BigEndian>(self.src_y_position));
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Unknown(i32),
    // core spec
    Raw,
    CopyRect,
    Rre,
    Hextile,
    Zrle,
    Cursor,
    DesktopSize,

    // extensions
    ExtendedKeyEvent,
}

impl Message for Encoding {
    fn read_from<R: Read>(reader: &mut R) -> Result<Encoding> {
        let encoding = try!(reader.read_i32::<BigEndian>());
        match encoding {
            0    => Ok(Encoding::Raw),
            1    => Ok(Encoding::CopyRect),
            2    => Ok(Encoding::Rre),
            5    => Ok(Encoding::Hextile),
            16   => Ok(Encoding::Zrle),
            -239 => Ok(Encoding::Cursor),
            -223 => Ok(Encoding::DesktopSize),
            -258 => Ok(Encoding::ExtendedKeyEvent),
            n    => Ok(Encoding::Unknown(n))
        }
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        let encoding = match self {
            &Encoding::Raw => 0,
            &Encoding::CopyRect => 1,
            &Encoding::Rre => 2,
            &Encoding::Hextile => 5,
            &Encoding::Zrle => 16,
            &Encoding::Cursor => -239,
            &Encoding::DesktopSize => -223,
            &Encoding::ExtendedKeyEvent => -258,
            &Encoding::Unknown(n) => n
        };
        try!(writer.write_i32::<BigEndian>(encoding));
        Ok(())
    }
}

#[derive(Debug)]
pub enum C2S {
    // core spec
    SetPixelFormat(PixelFormat),
    SetEncodings(Vec<Encoding>),
    FramebufferUpdateRequest {
        incremental: bool,
        x_position:  u16,
        y_position:  u16,
        width:       u16,
        height:      u16,
    },
    KeyEvent {
        down:        bool,
        key:         u32,
    },
    PointerEvent {
        button_mask: u8,
        x_position:  u16,
        y_position:  u16
    },
    CutText(String),

    // extensions
    ExtendedKeyEvent {
        down:        bool,
        keysym:      u32,
        keycode:     u32,
    },
}

impl Message for C2S {
    fn read_from<R: Read>(reader: &mut R) -> Result<C2S> {
        let message_type =
            match reader.read_u8() {
                Err(ref e) if e.kind() == IoErrorKind::UnexpectedEof =>
                    return Err(Error::Disconnected),
                result => try!(result)
            };
        match message_type {
            0 => {
                try!(reader.read_exact(&mut [0u8; 3]));
                Ok(C2S::SetPixelFormat(try!(PixelFormat::read_from(reader))))
            },
            2 => {
                try!(reader.read_exact(&mut [0u8; 1]));
                let count = try!(reader.read_u16::<BigEndian>());
                let mut encodings = Vec::new();
                for _ in 0..count {
                    encodings.push(try!(Encoding::read_from(reader)));
                }
                Ok(C2S::SetEncodings(encodings))
            },
            3 => {
                Ok(C2S::FramebufferUpdateRequest {
                    incremental: try!(reader.read_u8()) != 0,
                    x_position:  try!(reader.read_u16::<BigEndian>()),
                    y_position:  try!(reader.read_u16::<BigEndian>()),
                    width:       try!(reader.read_u16::<BigEndian>()),
                    height:      try!(reader.read_u16::<BigEndian>())
                })
            },
            4 => {
                let down = try!(reader.read_u8()) != 0;
                try!(reader.read_exact(&mut [0u8; 2]));
                let key = try!(reader.read_u32::<BigEndian>());
                Ok(C2S::KeyEvent { down: down, key: key })
            },
            5 => {
                Ok(C2S::PointerEvent {
                    button_mask: try!(reader.read_u8()),
                    x_position:  try!(reader.read_u16::<BigEndian>()),
                    y_position:  try!(reader.read_u16::<BigEndian>())
                })
            },
            6 => {
                try!(reader.read_exact(&mut [0u8; 3]));
                Ok(C2S::CutText(try!(String::read_from(reader))))
            },
            255 => {
                let submessage_type = try!(reader.read_u8());
                match submessage_type {
                    0 => {
                        let down = try!(reader.read_u16::<BigEndian>()) != 0;
                        let keysym = try!(reader.read_u32::<BigEndian>());
                        let keycode = try!(reader.read_u32::<BigEndian>());
                        Ok(C2S::ExtendedKeyEvent { down: down, keysym: keysym, keycode: keycode })
                    }
                    _ => Err(Error::Unexpected("server to client QEMU submessage type"))
                }
            }
            _ => Err(Error::Unexpected("client to server message type"))
        }
    }
    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        match self {
            &C2S::SetPixelFormat(ref pixel_format) => {
                try!(writer.write_u8(0));
                try!(writer.write_all(&[0u8; 3]));
                try!(PixelFormat::write_to(pixel_format, writer));
            },
            &C2S::SetEncodings(ref encodings) => {
                try!(writer.write_u8(2));
                try!(writer.write_all(&[0u8; 1]));
                try!(writer.write_u16::<BigEndian>(encodings.len() as u16)); // TODO: check?
                for encoding in encodings {
                    try!(Encoding::write_to(encoding, writer));
                }
            },
            &C2S::FramebufferUpdateRequest { incremental, x_position, y_position, width, height } => {
                try!(writer.write_u8(3));
                try!(writer.write_u8(if incremental { 1 } else { 0 }));
                try!(writer.write_u16::<BigEndian>(x_position));
                try!(writer.write_u16::<BigEndian>(y_position));
                try!(writer.write_u16::<BigEndian>(width));
                try!(writer.write_u16::<BigEndian>(height));
            },
            &C2S::KeyEvent { down, key } => {
                try!(writer.write_u8(4));
                try!(writer.write_u8(if down { 1 } else { 0 }));
                try!(writer.write_all(&[0u8; 2]));
                try!(writer.write_u32::<BigEndian>(key));
            },
            &C2S::PointerEvent { button_mask, x_position, y_position } => {
                try!(writer.write_u8(5));
                try!(writer.write_u8(button_mask));
                try!(writer.write_u16::<BigEndian>(x_position));
                try!(writer.write_u16::<BigEndian>(y_position));
            },
            &C2S::CutText(ref text) => {
                try!(String::write_to(text, writer));
            }
            &C2S::ExtendedKeyEvent { down, keysym, keycode } => {
                try!(writer.write_u8(255));
                try!(writer.write_u8(0));
                try!(writer.write_u16::<BigEndian>(if down { 1 } else { 0 }));
                try!(writer.write_u32::<BigEndian>(keysym));
                try!(writer.write_u32::<BigEndian>(keycode));
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct RectangleHeader {
    pub x_position: u16,
    pub y_position: u16,
    pub width:      u16,
    pub height:     u16,
    pub encoding:   Encoding,
}

impl Message for RectangleHeader {
    fn read_from<R: Read>(reader: &mut R) -> Result<RectangleHeader> {
        Ok(RectangleHeader {
            x_position: try!(reader.read_u16::<BigEndian>()),
            y_position: try!(reader.read_u16::<BigEndian>()),
            width:      try!(reader.read_u16::<BigEndian>()),
            height:     try!(reader.read_u16::<BigEndian>()),
            encoding:   try!(Encoding::read_from(reader))
        })
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        try!(writer.write_u16::<BigEndian>(self.x_position));
        try!(writer.write_u16::<BigEndian>(self.y_position));
        try!(writer.write_u16::<BigEndian>(self.width));
        try!(writer.write_u16::<BigEndian>(self.height));
        try!(Encoding::write_to(&self.encoding, writer));
        Ok(())
    }
}

#[derive(Debug)]
pub struct Colour {
    pub red:   u16,
    pub green: u16,
    pub blue:  u16
}

impl Message for Colour {
    fn read_from<R: Read>(reader: &mut R) -> Result<Colour> {
        Ok(Colour {
            red:   try!(reader.read_u16::<BigEndian>()),
            green: try!(reader.read_u16::<BigEndian>()),
            blue:  try!(reader.read_u16::<BigEndian>())
        })
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        try!(writer.write_u16::<BigEndian>(self.red));
        try!(writer.write_u16::<BigEndian>(self.green));
        try!(writer.write_u16::<BigEndian>(self.blue));
        Ok(())
    }
}

#[derive(Debug)]
pub enum S2C {
    // core spec
    FramebufferUpdate {
        count:        u16,
        // Vec<RectangleHeader> has to be read out manually
    },
    SetColourMapEntries {
        first_colour: u16,
        colours:      Vec<Colour>
    },
    Bell,
    CutText(String),
    // extensions
}

impl Message for S2C {
    fn read_from<R: Read>(reader: &mut R) -> Result<S2C> {
        let message_type =
            match reader.read_u8() {
                Err(ref e) if e.kind() == IoErrorKind::UnexpectedEof =>
                    return Err(Error::Disconnected),
                result => try!(result)
            };
        match message_type {
            0 => {
                try!(reader.read_exact(&mut [0u8; 1]));
                Ok(S2C::FramebufferUpdate {
                    count: try!(reader.read_u16::<BigEndian>())
                })
            },
            1 => {
                try!(reader.read_exact(&mut [0u8; 1]));
                let first_colour = try!(reader.read_u16::<BigEndian>());
                let count = try!(reader.read_u16::<BigEndian>());
                let mut colours = Vec::new();
                for _ in 0..count {
                    colours.push(try!(Colour::read_from(reader)));
                }
                Ok(S2C::SetColourMapEntries { first_colour: first_colour, colours: colours })
            },
            2 => {
                Ok(S2C::Bell)
            },
            3 => {
                try!(reader.read_exact(&mut [0u8; 3]));
                Ok(S2C::CutText(try!(String::read_from(reader))))
            },
            _ => Err(Error::Unexpected("server to client message type"))
        }
    }

    fn write_to<W: Write>(&self, writer: &mut W) -> Result<()> {
        match self {
            &S2C::FramebufferUpdate { count } => {
                try!(writer.write_u8(0));
                try!(writer.write_all(&[0u8; 1]));
                try!(writer.write_u16::<BigEndian>(count));
            },
            &S2C::SetColourMapEntries { first_colour, ref colours } => {
                try!(writer.write_u8(1));
                try!(writer.write_all(&[0u8; 1]));
                try!(writer.write_u16::<BigEndian>(first_colour));
                for colour in colours {
                    try!(Colour::write_to(colour, writer));
                }
            },
            &S2C::Bell => {
                try!(writer.write_u8(2));
            },
            &S2C::CutText(ref text) => {
                try!(writer.write_u8(3));
                try!(writer.write_all(&[0u8; 3]));
                try!(String::write_to(text, writer));
            }
        }
        Ok(())
    }
}
