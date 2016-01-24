#[macro_use] extern crate log;
// TODO: https://github.com/BurntSushi/byteorder/pull/40
extern crate byteorder;

mod protocol;

pub use protocol::{Error, Result, Version, PixelFormat};

use std::io::{Cursor, Read, Write};
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
                            debug!("<- ...pixels");
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
                    vec![]
                } else {
                    vec![security_type]
                }
            },
            _ => {
                let security_types = try!(protocol::SecurityTypes::read_from(&mut stream));
                debug!("<- {:?}", security_types);
                security_types.0
            }
        };

        if security_types.len() == 0 {
            let reason = try!(String::read_from(&mut stream));
            debug!("<- {:?}", reason);
            return Err(Error::Server(reason))
        }

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

pub struct Proxy {
    c2s_thread: thread::JoinHandle<Result<()>>,
    s2c_thread: thread::JoinHandle<Result<()>>,
}

impl Proxy {
    pub fn from_tcp_streams(mut server_stream: TcpStream, mut client_stream: TcpStream) ->
            Result<Proxy> {
        let server_version = try!(protocol::Version::read_from(&mut server_stream));
        debug!("c<-s {:?}", server_version);
        try!(protocol::Version::write_to(&server_version, &mut client_stream));

        let client_version = try!(protocol::Version::read_from(&mut client_stream));
        debug!("c->s {:?}", client_version);
        try!(protocol::Version::write_to(&client_version, &mut server_stream));

        fn security_type_supported(security_type: &protocol::SecurityType) -> bool {
            match security_type {
                &protocol::SecurityType::None => true,
                security_type => {
                    warn!("security type {:?} is not supported", security_type);
                    false
                }
            }
        }

        let security_types = match client_version {
            Version::Rfb33 => {
                let mut security_type = try!(protocol::SecurityType::read_from(&mut server_stream));
                debug!("!<-s SecurityType::{:?}", security_type);

                // Filter out security types we can't handle
                if !security_type_supported(&security_type) {
                    security_type = protocol::SecurityType::Invalid
                }

                debug!("c<-! SecurityType::{:?}", security_type);
                try!(protocol::SecurityType::write_to(&security_type, &mut client_stream));

                if security_type == protocol::SecurityType::Invalid {
                    vec![]
                } else {
                    vec![security_type]
                }
            },
            _ => {
                let mut security_types =
                    try!(protocol::SecurityTypes::read_from(&mut server_stream));
                debug!("!<-s {:?}", security_types);

                // Filter out security types we can't handle
                security_types.0.retain(security_type_supported);

                debug!("c<-! {:?}", security_types);
                try!(protocol::SecurityTypes::write_to(&security_types, &mut client_stream));

                security_types.0
            }
        };

        if security_types.len() == 0 {
            let reason = try!(String::read_from(&mut server_stream));
            debug!("c<-s {:?}", reason);
            try!(String::write_to(&reason, &mut client_stream));

            return Err(Error::Server(reason))
        }

        let used_security_type = match client_version {
            Version::Rfb33 => security_types[0],
            _ => {
                let used_security_type =
                    try!(protocol::SecurityType::read_from(&mut client_stream));
                debug!("c->s SecurityType::{:?}", used_security_type);
                try!(protocol::SecurityType::write_to(&used_security_type, &mut server_stream));

                used_security_type
            }
        };

        let mut skip_security_result = false;
        match &(used_security_type, client_version) {
            &(protocol::SecurityType::None, Version::Rfb33) |
            &(protocol::SecurityType::None, Version::Rfb37) => skip_security_result = true,
            _ => ()
        }

        if !skip_security_result {
            let security_result = try!(protocol::SecurityResult::read_from(&mut server_stream));
            debug!("c<-s SecurityResult::{:?}", security_result);
            try!(protocol::SecurityResult::write_to(&security_result, &mut client_stream));

            if security_result == protocol::SecurityResult::Failed {
                match client_version {
                    Version::Rfb33 | Version::Rfb37 =>
                        return Err(Error::AuthenticationFailure(String::from(""))),
                    Version::Rfb38 => {
                        let reason = try!(String::read_from(&mut server_stream));
                        debug!("c<-s {:?}", reason);
                        try!(String::write_to(&reason, &mut client_stream));
                        return Err(Error::AuthenticationFailure(reason))
                    }
                }
            }
        }

        let client_init = try!(protocol::ClientInit::read_from(&mut client_stream));
        debug!("c->s {:?}", client_init);
        try!(protocol::ClientInit::write_to(&client_init, &mut server_stream));

        let server_init = try!(protocol::ServerInit::read_from(&mut server_stream));
        debug!("c<-s {:?}", server_init);
        try!(protocol::ServerInit::write_to(&server_init, &mut client_stream));

        let (mut c2s_server_stream, mut c2s_client_stream) =
            (server_stream.try_clone().unwrap(), client_stream.try_clone().unwrap());
        let (mut s2c_server_stream, mut s2c_client_stream) =
            (server_stream.try_clone().unwrap(), client_stream.try_clone().unwrap());

        fn forward_c2s(server_stream: &mut TcpStream, client_stream: &mut TcpStream) ->
                Result<()> {
            fn encoding_supported(encoding: &protocol::Encoding) -> bool {
                match encoding {
                    &protocol::Encoding::Raw |
                    &protocol::Encoding::CopyRect |
                    &protocol::Encoding::Zrle |
                    &protocol::Encoding::Cursor |
                    &protocol::Encoding::DesktopSize => true,
                    encoding => {
                        warn!("encoding {:?} is not supported", encoding);
                        false
                    }
                }
            }

            loop {
                let mut message = try!(protocol::C2S::read_from(client_stream));
                match message {
                    protocol::C2S::SetEncodings(ref mut encodings) => {
                        debug!("c->! SetEncodings({:?})", encodings);

                        // Filter out encodings we can't handle
                        encodings.retain(encoding_supported);

                        debug!("!->s SetEncodings({:?})", encodings);
                    },
                    protocol::C2S::SetPixelFormat(_) => {
                        // There is an inherent race condition in the VNC protocol (I think)
                        // between SetPixelFormat and FramebufferUpdate and I've no idea
                        // how to handle it properly, so defer for now.
                        panic!("proxying SetPixelFormat is not implemented!")
                    },
                    ref message => debug!("c->s {:?}", message)
                }
                try!(protocol::C2S::write_to(&message, server_stream))
            }
        }

        fn forward_s2c(server_stream: &mut TcpStream, client_stream: &mut TcpStream,
                       format: PixelFormat) ->
                Result<()> {

            loop {
                let mut buffer_stream = Cursor::new(Vec::new());

                let message = try!(protocol::S2C::read_from(server_stream));
                debug!("c<-s {:?}", message);
                try!(protocol::S2C::write_to(&message, &mut buffer_stream));

                match message {
                    protocol::S2C::FramebufferUpdate { count } => {
                        for _ in 0..count {
                            let rectangle = try!(protocol::Rectangle::read_from(server_stream));
                            debug!("c<-s {:?}", rectangle);
                            try!(protocol::Rectangle::write_to(&rectangle, &mut buffer_stream));

                            match rectangle.encoding {
                                protocol::Encoding::Raw => {
                                    let mut pixels = vec![0; (rectangle.width as usize) *
                                                             (rectangle.height as usize) *
                                                             (format.bits_per_pixel as usize / 8)];
                                    try!(server_stream.read_exact(&mut pixels));
                                    debug!("c<-s ...raw pixels");
                                    try!(buffer_stream.write_all(&pixels));
                                },
                                protocol::Encoding::CopyRect => {
                                    let copy_rect =
                                        try!(protocol::CopyRect::read_from(server_stream));
                                    debug!("c<-s {:?}", copy_rect);
                                    try!(protocol::CopyRect::write_to(&copy_rect,
                                                                      &mut buffer_stream));
                                },
                                protocol::Encoding::Zrle => {
                                    let zrle = try!(Vec::<u8>::read_from(server_stream));
                                    debug!("c<-s ...ZRLE pixels");
                                    try!(Vec::<u8>::write_to(&zrle, &mut buffer_stream));
                                }
                                protocol::Encoding::Cursor => {
                                    let mut pixels    = vec![0; (rectangle.width as usize) *
                                                                (rectangle.height as usize) *
                                                                (format.bits_per_pixel as usize / 8)];
                                    try!(server_stream.read_exact(&mut pixels));
                                    try!(buffer_stream.write_all(&pixels));
                                    let mut mask_bits = vec![0; ((rectangle.width as usize + 7) / 8) *
                                                                (rectangle.height as usize)];
                                    try!(server_stream.read_exact(&mut mask_bits));
                                    try!(buffer_stream.write_all(&mask_bits));
                                },
                                protocol::Encoding::DesktopSize => (),
                                _ => return Err(Error::UnexpectedValue("encoding"))
                            }
                        }
                    },
                    _ => ()
                }

                let buffer = buffer_stream.into_inner();
                try!(client_stream.write_all(&buffer));
            }
        }

        Ok(Proxy {
            c2s_thread: thread::spawn(move || {
                let result = forward_c2s(&mut c2s_server_stream, &mut c2s_client_stream);
                let _ = c2s_server_stream.shutdown(Shutdown::Both);
                let _ = c2s_client_stream.shutdown(Shutdown::Both);
                result
            }),
            s2c_thread: thread::spawn(move || {
                let result = forward_s2c(&mut s2c_server_stream, &mut s2c_client_stream,
                                         server_init.pixel_format);
                let _ = s2c_server_stream.shutdown(Shutdown::Both);
                let _ = s2c_client_stream.shutdown(Shutdown::Both);
                result
            })
        })
    }

    pub fn join(self) -> Result<()> {
        let c2s_result = self.c2s_thread.join().unwrap();
        let s2c_result = self.s2c_thread.join().unwrap();
        match c2s_result.and(s2c_result) {
            Err(Error::Disconnected) => Ok(()),
            result => result
        }
    }
}
