use std::io::Read;
use std::net::{TcpStream, Shutdown};
use std::thread;
use std::sync::{Arc, Mutex};
use std::sync::mpsc::{channel, Sender, Receiver, TryRecvError};
use byteorder::{BigEndian, ReadBytesExt};
use ::{zrle, protocol, Rect, Colour, Error, Result};
use protocol::Message;

#[derive(Debug)]
pub enum AuthMethod {
    None,
    /* more to come */
    #[doc(hidden)]
    __Nonexhaustive,
}

#[derive(Debug)]
pub enum AuthChoice {
    None,
    /* more to come */
    #[doc(hidden)]
    __Nonexhaustive,
}

#[derive(Debug)]
pub enum Event {
    Disconnected(Option<Error>),
    Resize(u16, u16),
    SetColourMap { first_colour: u16, colours: Vec<Colour> },
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
            Err(_) => return Ok(false)
        }
    })
}

impl Event {
    fn pump_one(stream: &mut TcpStream, format: &Arc<Mutex<protocol::PixelFormat>>,
                zrle_decoder: &mut zrle::Decoder, tx_events: &mut Sender<Event>) -> Result<bool> {
        let packet =
            match protocol::S2C::read_from(stream) {
                Ok(packet) => packet,
                Err(Error::Disconnected) => {
                    send_or_return!(tx_events, Event::Disconnected(None));
                    return Ok(false)
                },
                Err(error) => return Err(error)
            };
        debug!("<- {:?}", packet);

        let format = *format.lock().unwrap();
        match packet {
            protocol::S2C::SetColourMapEntries { first_colour, colours } => {
                send_or_return!(tx_events, Event::SetColourMap {
                    first_colour: first_colour, colours: colours
                })
            },
            protocol::S2C::FramebufferUpdate { count } => {
                for _ in 0..count {
                    let rectangle = try!(protocol::Rectangle::read_from(stream));
                    debug!("<- {:?}", rectangle);

                    let dst = Rect {
                        left:   rectangle.x_position,
                        top:    rectangle.y_position,
                        width:  rectangle.width,
                        height: rectangle.height
                    };
                    match rectangle.encoding {
                        protocol::Encoding::Raw => {
                            let mut pixels = vec![0; (rectangle.width as usize) *
                                                     (rectangle.height as usize) *
                                                     (format.bits_per_pixel as usize / 8)];
                            try!(stream.read_exact(&mut pixels));
                            debug!("<- ...pixels");
                            send_or_return!(tx_events, Event::PutPixels(dst, pixels))
                        },
                        protocol::Encoding::CopyRect => {
                            let copy_rect = try!(protocol::CopyRect::read_from(stream));
                            let src = Rect {
                                left:   copy_rect.src_x_position,
                                top:    copy_rect.src_y_position,
                                width:  rectangle.width,
                                height: rectangle.height
                            };
                            send_or_return!(tx_events, Event::CopyPixels { src: src, dst: dst })
                        },
                        protocol::Encoding::Zrle => {
                            let length = try!(stream.read_u32::<BigEndian>());
                            let mut data = vec![0; length as usize];
                            try!(stream.read_exact(&mut data));
                            debug!("<- ...compressed pixels");
                            let result = try!(zrle_decoder.decode(format, dst, &data,
                                |tile, pixels| {
                                    send_or_return!(tx_events, Event::PutPixels(tile, pixels));
                                    Ok(true)
                                }));
                            if !result { return Ok(false) }
                        }
                        protocol::Encoding::Cursor => {
                            let mut pixels    = vec![0; (rectangle.width as usize) *
                                                        (rectangle.height as usize) *
                                                        (format.bits_per_pixel as usize / 8)];
                            try!(stream.read_exact(&mut pixels));
                            let mut mask_bits = vec![0; ((rectangle.width as usize + 7) / 8) *
                                                        (rectangle.height as usize)];
                            try!(stream.read_exact(&mut mask_bits));
                            send_or_return!(tx_events, Event::SetCursor {
                                size:      (rectangle.width, rectangle.height),
                                hotspot:   (rectangle.x_position, rectangle.y_position),
                                pixels:    pixels,
                                mask_bits: mask_bits
                            })
                        },
                        protocol::Encoding::DesktopSize => {
                            send_or_return!(tx_events,
                                Event::Resize(rectangle.width, rectangle.height))
                        }
                        _ => return Err(Error::Unexpected("encoding"))
                    };
                }
            },
            protocol::S2C::Bell =>
                send_or_return!(tx_events, Event::Bell),
            protocol::S2C::CutText(text) =>
                send_or_return!(tx_events, Event::Clipboard(text))
        };
        Ok(true)
    }

    fn pump(mut stream: TcpStream, format: Arc<Mutex<protocol::PixelFormat>>) -> Receiver<Event> {
        let (mut tx_events, rx_events) = channel();
        thread::spawn(move || {
            let mut zrle_decoder = zrle::Decoder::new();
            loop {
                match Event::pump_one(&mut stream, &format, &mut zrle_decoder, &mut tx_events) {
                    Ok(true) => (),
                    Ok(false) => break,
                    Err(error) => {
                        let _ = tx_events.send(Event::Disconnected(Some(error)));
                        break
                    }
                }
            }
        });
        rx_events
    }
}

pub struct Client {
    stream:  TcpStream,
    events:  Receiver<Event>,
    name:    String,
    size:    (u16, u16),
    format:  Arc<Mutex<protocol::PixelFormat>>
}

impl Client {
    pub fn from_tcp_stream<Auth>(mut stream: TcpStream, shared: bool,
                                 auth: Auth) -> Result<Client>
            where Auth: FnOnce(&[AuthMethod]) -> Option<AuthChoice> {
        let version = try!(protocol::Version::read_from(&mut stream));
        debug!("<- Version::{:?}", version);
        debug!("-> Version::{:?}", version);
        try!(protocol::Version::write_to(&version, &mut stream));

        let security_types = match version {
            protocol::Version::Rfb33 => {
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
            protocol::Version::Rfb33 => (),
            _ => {
                let used_security_type = match auth_choice {
                    AuthChoice::None => protocol::SecurityType::None,
                    AuthChoice::__Nonexhaustive => unreachable!()
                };
                debug!("-> SecurityType::{:?}", used_security_type);
                try!(protocol::SecurityType::write_to(&used_security_type, &mut stream));
            }
        }

        let mut skip_security_result = false;
        match &(auth_choice, version) {
            &(AuthChoice::None, protocol::Version::Rfb33) |
            &(AuthChoice::None, protocol::Version::Rfb37) => skip_security_result = true,
            _ => ()
        }

        if !skip_security_result {
            match try!(protocol::SecurityResult::read_from(&mut stream)) {
                protocol::SecurityResult::Succeeded => (),
                protocol::SecurityResult::Failed => {
                    match version {
                        protocol::Version::Rfb33 |
                        protocol::Version::Rfb37 =>
                            return Err(Error::AuthenticationFailure(String::from(""))),
                        protocol::Version::Rfb38 => {
                            let reason = try!(String::read_from(&mut stream));
                            debug!("<- {:?}", reason);
                            return Err(Error::AuthenticationFailure(reason))
                        }
                    }
                }
            }
        }

        let client_init = protocol::ClientInit { shared: shared };
        debug!("-> {:?}", client_init);
        try!(protocol::ClientInit::write_to(&client_init, &mut stream));

        let server_init = try!(protocol::ServerInit::read_from(&mut stream));
        debug!("<- {:?}", server_init);

        let format = Arc::new(Mutex::new(server_init.pixel_format));
        let events = Event::pump(stream.try_clone().unwrap(), format.clone());

        Ok(Client {
            stream:  stream,
            events:  events,
            name:    server_init.name,
            size:    (server_init.framebuffer_width, server_init.framebuffer_height),
            format:  format
        })
    }

    pub fn name(&self) -> &str { &self.name }
    pub fn size(&self) -> (u16, u16) { self.size }
    pub fn format(&self) -> protocol::PixelFormat { *self.format.lock().unwrap() }

    pub fn set_encodings(&mut self, encodings: &[protocol::Encoding]) -> Result<()> {
        let set_encodings = protocol::C2S::SetEncodings(Vec::from(encodings));
        debug!("-> {:?}", set_encodings);
        try!(protocol::C2S::write_to(&set_encodings, &mut self.stream));
        Ok(())
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

    // Note that due to inherent weaknesses of the VNC protocol, this
    // function is prone to race conditions that break the connection framing.
    // The ZRLE encoding is self-delimiting and if both the client and server
    // support and use it, there can be no race condition, but we currently don't.
    pub fn set_format(&mut self, format: protocol::PixelFormat) -> Result<()> {
        // Request (and discard) one full update to try and ensure that there
        // are no FramebufferUpdate's in the buffers somewhere.
        // This is not fully robust though (and cannot possibly be).
        let _ = self.poll_iter().count(); // drain it
        let framebuffer_rect = Rect { left: 0, top: 0, width: self.size.0, height: self.size.1 };
        try!(self.request_update(framebuffer_rect, false));
        'outer: loop {
            for event in self.poll_iter() {
                match event {
                    Event::PutPixels(rect, _) if rect == framebuffer_rect => break 'outer,
                    _ => ()
                }
            }
        }

        // Since VNC is fully client-driven, by this point the event thread is stuck
        // waiting for the next message and the server is not sending us anything,
        // so it's safe to switch to the new pixel format.
        let set_pixel_format = protocol::C2S::SetPixelFormat(format);
        debug!("-> {:?}", set_pixel_format);
        try!(protocol::C2S::write_to(&set_pixel_format, &mut self.stream));
        *self.format.lock().unwrap() = format;

        Ok(())
    }

    #[doc(hidden)]
    pub fn poke_qemu(&mut self) -> Result<()> {
        let set_pixel_format = protocol::C2S::SetPixelFormat(*self.format.lock().unwrap());
        debug!("-> {:?}", set_pixel_format);
        try!(protocol::C2S::write_to(&set_pixel_format, &mut self.stream));
        Ok(())
    }

    pub fn poll_event(&mut self) -> Option<Event> {
        match self.events.try_recv() {
            Err(TryRecvError::Empty) |
            Err(TryRecvError::Disconnected) => None,
            Ok(Event::Resize(width, height)) => {
                self.size = (width, height);
                Some(Event::Resize(width, height))
            }
            Ok(event) => Some(event)
        }
    }

    pub fn poll_iter(&mut self) -> EventPollIterator {
        EventPollIterator { client: self }
    }

    pub fn disconnect(self) -> Result<()> {
        try!(self.stream.shutdown(Shutdown::Both));
        Ok(())
    }
}

pub struct EventPollIterator<'a> {
    client: &'a mut Client
}

impl<'a> Iterator for EventPollIterator<'a> {
    type Item = Event;

    fn next(&mut self) -> Option<Self::Item> { self.client.poll_event() }
}
