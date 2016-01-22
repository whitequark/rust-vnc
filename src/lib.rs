#![feature(read_exact)]

#[macro_use] extern crate log;
extern crate byteorder;

mod protocol;

pub use protocol::{Error, Result, Version, PixelFormat};

use std::io::Read;
use std::net::{TcpStream, Shutdown};
use std::thread;
use std::sync::mpsc::{channel, Sender, Receiver, TryRecvError};
use protocol::Message;

#[derive(Debug)]
pub enum AuthMethod {
    None,
    /* more to come */
    Unused
}

#[derive(Debug)]
pub enum AuthChoice {
    None,
    /* more to come */
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Rect {
    pub left:   u16,
    pub top:    u16,
    pub width:  u16,
    pub height: u16
}

#[derive(Debug)]
pub enum ClientEvent {
    Disconnected,
    Resize(u16, u16),
    PutPixels(Rect, Vec<u8>),
    CopyPixels { src: Rect, dst: Rect },
    SetCursor { size: (u16, u16), hotspot: (u16, u16), pixels: Vec<u8>, mask_bits: Vec<u8> },
    Clipboard(String),
    Bell,
}

macro_rules! send_or_return {
    ($chan:expr, $data:expr) => ({
        match $chan.send($data) {
            Ok(()) => (),
            Err(_) => return Err(Error::UnexpectedEOF)
        }
    })
}

impl ClientEvent {
    fn pump_one(stream: &mut TcpStream, format: &protocol::PixelFormat,
                tx_events: &mut Sender<ClientEvent>) -> Result<()> {
        let packet = try!(protocol::S2C::read_from(stream));
        debug!("<- {:?}", packet);
        match packet {
            protocol::S2C::FramebufferUpdate { count } => {
                for _ in 0..count {
                    let rectangle = try!(protocol::Rectangle::read_from(stream));
                    debug!("<- {:?}", rectangle);
                    let event = match rectangle.encoding {
                        protocol::Encoding::Raw => {
                            let mut pixels = vec![0; (rectangle.width as usize) *
                                                     (rectangle.height as usize) *
                                                     (format.bits_per_pixel as usize / 8)];
                            try!(stream.read_exact(&mut pixels));
                            ClientEvent::PutPixels(Rect {
                                left:   rectangle.x_position,
                                top:    rectangle.y_position,
                                width:  rectangle.width,
                                height: rectangle.height
                            }, pixels)
                        },
                        protocol::Encoding::CopyRect => {
                            let copy_rect = try!(protocol::CopyRect::read_from(stream));
                            let src = Rect {
                                left:   copy_rect.src_x_position,
                                top:    copy_rect.src_y_position,
                                width:  rectangle.width,
                                height: rectangle.height
                            };
                            let dst = Rect {
                                left:   rectangle.x_position,
                                top:    rectangle.y_position,
                                width:  rectangle.width,
                                height: rectangle.height
                            };
                            ClientEvent::CopyPixels { src: src, dst: dst }
                        },
                        protocol::Encoding::Cursor => {
                            let mut pixels    = vec![0; (rectangle.width as usize) *
                                                        (rectangle.height as usize) *
                                                        (format.bits_per_pixel as usize / 8)];
                            try!(stream.read_exact(&mut pixels));
                            let mut mask_bits = vec![0; ((rectangle.width as usize + 7) / 8) *
                                                        (rectangle.height as usize)];
                            try!(stream.read_exact(&mut mask_bits));
                            ClientEvent::SetCursor {
                                size:      (rectangle.width, rectangle.height),
                                hotspot:   (rectangle.x_position, rectangle.y_position),
                                pixels:    pixels,
                                mask_bits: mask_bits
                            }
                        },
                        protocol::Encoding::DesktopSize =>
                            ClientEvent::Resize(rectangle.width, rectangle.height),
                        _ => return Err(Error::UnexpectedValue("encoding"))
                    };
                    send_or_return!(tx_events, event)
                }
            },
            protocol::S2C::Bell =>
                send_or_return!(tx_events, ClientEvent::Bell),
            protocol::S2C::CutText(text) =>
                send_or_return!(tx_events, ClientEvent::Clipboard(text)),
            _ => return Err(Error::UnexpectedValue("server to client packet"))
        };
        Ok(())
    }

    fn pump(mut stream: TcpStream, format: protocol::PixelFormat) -> Receiver<ClientEvent> {
        let (mut tx_events, rx_events) = channel();
        thread::spawn(move || {
            loop {
                match ClientEvent::pump_one(&mut stream, &format, &mut tx_events) {
                    Ok(()) => (),
                    Err(Error::UnexpectedEOF) => break,
                    Err(error) => panic!("cannot pump VNC client events: {}", error)
                }
            }
        });
        rx_events
    }
}

pub struct ClientBuilder {
    shared:       bool,
    copy_rect:    bool,
    set_cursor:   bool,
    resize:       bool,
}

impl ClientBuilder {
    pub fn new() -> ClientBuilder {
        ClientBuilder {
            shared:       false,
            copy_rect:    false,
            set_cursor:   false,
            resize:       false,
        }
    }

    pub fn shared    (mut self, value: bool) -> ClientBuilder { self.shared = value; self }
    pub fn copy_rect (mut self, value: bool) -> ClientBuilder { self.copy_rect = value; self }
    pub fn set_cursor(mut self, value: bool) -> ClientBuilder { self.set_cursor = value; self }
    pub fn resize    (mut self, value: bool) -> ClientBuilder { self.resize = value; self }

    pub fn from_tcp_stream<Auth>(self, mut stream: TcpStream, auth: Auth) -> Result<Client>
            where Auth: FnOnce(&[AuthMethod]) -> Option<AuthChoice> {
        let version = try!(protocol::Version::read_from(&mut stream));
        debug!("<- Version::{:?}", version);
        debug!("-> Version::{:?}", version);
        try!(protocol::Version::write_to(&version, &mut stream));

        let security_types = match version {
            Version::Rfb33 => {
                let security_type = try!(protocol::SecurityType::read_from(&mut stream));
                debug!("<- SecurityType::{:?}", security_type);
                if security_type == protocol::SecurityType::Invalid {
                    let reason = try!(String::read_from(&mut stream));
                    debug!("<- {:?}", reason);
                    return Err(Error::Server(reason))
                }
                vec![security_type]
            },
            _ => {
                let security_types = try!(protocol::SecurityTypes::read_from(&mut stream));
                debug!("<- {:?}", security_types);
                if security_types.0.len() == 0 {
                    let reason = try!(String::read_from(&mut stream));
                    debug!("<- {:?}", reason);
                    return Err(Error::Server(reason))
                }
                security_types.0
            }
        };

        let mut auth_methods = Vec::new();
        for security_type in security_types {
            match security_type {
                protocol::SecurityType::None =>
                    auth_methods.push(AuthMethod::None),
                _ => ()
            }
        }

        let auth_choice = try!(auth(&auth_methods).ok_or(Error::AuthenticationUnavailable));

        match version {
            Version::Rfb33 => (),
            _ => {
                let used_security_type = match auth_choice {
                    AuthChoice::None => protocol::SecurityType::None,
                };
                debug!("-> SecurityType::{:?}", used_security_type);
                try!(protocol::SecurityType::write_to(&used_security_type, &mut stream));
            }
        }

        let mut skip_security_result = false;
        match &(auth_choice, version) {
            &(AuthChoice::None, Version::Rfb33) |
            &(AuthChoice::None, Version::Rfb37) => skip_security_result = true,
            _ => ()
        }

        if !skip_security_result {
            let security_result = try!(protocol::SecurityResult::read_from(&mut stream));
            if security_result == protocol::SecurityResult::Failed {
                match version {
                    Version::Rfb33 | Version::Rfb37 =>
                        return Err(Error::AuthenticationFailure(String::from(""))),
                    Version::Rfb38 => {
                        let reason = try!(String::read_from(&mut stream));
                        debug!("<- {:?}", reason);
                        return Err(Error::AuthenticationFailure(reason))
                    }
                }
            }
        }

        let client_init = protocol::ClientInit { shared: self.shared };
        debug!("-> {:?}", client_init);
        try!(protocol::ClientInit::write_to(&client_init, &mut stream));

        let server_init = try!(protocol::ServerInit::read_from(&mut stream));
        debug!("<- {:?}", server_init);

        let events = ClientEvent::pump(stream.try_clone().unwrap(),
                                       server_init.pixel_format.clone());

        let mut encodings = vec![protocol::Encoding::Raw];
        if self.copy_rect  { encodings.push(protocol::Encoding::CopyRect) }
        if self.set_cursor { encodings.push(protocol::Encoding::Cursor) }
        if self.resize     { encodings.push(protocol::Encoding::DesktopSize) }

        let set_encodings = protocol::C2S::SetEncodings(encodings);
        debug!("-> {:?}", set_encodings);
        try!(protocol::C2S::write_to(&set_encodings, &mut stream));

        Ok(Client {
            stream:  stream,
            events:  events,
            version: version,
            name:    server_init.name,
            size:    (server_init.framebuffer_width, server_init.framebuffer_height),
            format:  server_init.pixel_format
        })
    }
}

pub struct Client {
    stream:  TcpStream,
    events:  Receiver<ClientEvent>,
    version: Version,
    name:    String,
    size:    (u16, u16),
    format:  PixelFormat
}

impl Client {
    pub fn version(&self) -> Version { self.version }
    pub fn name(&self) -> &str { &self.name }
    pub fn size(&self) -> (u16, u16) { self.size }
    pub fn format(&self) -> PixelFormat { self.format.clone() }

    fn enable_encodings(&mut self, encodings: &[protocol::Encoding]) -> Result<()> {
        let set_encodings = protocol::C2S::SetEncodings(Vec::from(encodings));
        debug!("-> {:?}", set_encodings);
        try!(protocol::C2S::write_to(&set_encodings, &mut self.stream));
        Ok(())
    }

    pub fn enable_copy_pixels(&mut self) -> Result<()> {
        self.enable_encodings(&[protocol::Encoding::CopyRect])
    }

    pub fn enable_cursor(&mut self) -> Result<()> {
        self.enable_encodings(&[protocol::Encoding::Cursor])
    }

    pub fn enable_resize(&mut self) -> Result<()> {
        self.enable_encodings(&[protocol::Encoding::DesktopSize])
    }

    pub fn request_update(&mut self, rect: Rect, incremental: bool) -> Result<()> {
        let update_req = protocol::C2S::FramebufferUpdateRequest {
            incremental: incremental,
            x_position:  rect.left,
            y_position:  rect.top,
            width:       rect.width,
            height:      rect.height
        };
        trace!("-> {:?}", update_req);
        try!(protocol::C2S::write_to(&update_req, &mut self.stream));
        Ok(())
    }

    pub fn send_key_event(&mut self, down: bool, key: u32) -> Result<()> {
        let key_event = protocol::C2S::KeyEvent {
            down: down,
            key:  key
        };
        debug!("-> {:?}", key_event);
        try!(protocol::C2S::write_to(&key_event, &mut self.stream));
        Ok(())
    }

    pub fn send_pointer_event(&mut self, buttons: u8, x: u16, y: u16) -> Result<()> {
        let pointer_event = protocol::C2S::PointerEvent {
            button_mask: buttons,
            x_position:  x,
            y_position:  y
        };
        debug!("-> {:?}", pointer_event);
        try!(protocol::C2S::write_to(&pointer_event, &mut self.stream));
        Ok(())
    }

    pub fn update_clipboard(&mut self, text: &str) -> Result<()> {
        let cut_text = protocol::C2S::CutText(String::from(text));
        debug!("-> {:?}", cut_text);
        try!(protocol::C2S::write_to(&cut_text, &mut self.stream));
        Ok(())
    }

    pub fn poll_event(&mut self) -> Option<ClientEvent> {
        match self.events.try_recv() {
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(ClientEvent::Disconnected),
            Ok(ClientEvent::Resize(width, height)) => {
                self.size = (width, height);
                Some(ClientEvent::Resize(width, height))
            }
            Ok(event) => Some(event)
        }
    }

    pub fn poll_iter(&mut self) -> ClientEventPollIterator {
        ClientEventPollIterator { client: self }
    }

    pub fn disconnect(self) -> Result<()> {
        try!(self.stream.shutdown(Shutdown::Both));
        Ok(())
    }
}

pub struct ClientEventPollIterator<'a> {
    client:  &'a mut Client
}

impl<'a> Iterator for ClientEventPollIterator<'a> {
    type Item = ClientEvent;

    fn next(&mut self) -> Option<Self::Item> { self.client.poll_event() }
}
