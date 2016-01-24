use std::io::{Read, Write, Cursor};
use std::net::{TcpStream, Shutdown};
use std::thread;
use ::{Error, Result};
use protocol::{self, Message};

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
            protocol::Version::Rfb33 => {
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
            protocol::Version::Rfb33 => security_types[0],
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
            &(protocol::SecurityType::None, protocol::Version::Rfb33) |
            &(protocol::SecurityType::None, protocol::Version::Rfb37) =>
                skip_security_result = true,
            _ => ()
        }

        if !skip_security_result {
            let security_result = try!(protocol::SecurityResult::read_from(&mut server_stream));
            debug!("c<-s SecurityResult::{:?}", security_result);
            try!(protocol::SecurityResult::write_to(&security_result, &mut client_stream));

            if security_result == protocol::SecurityResult::Failed {
                match client_version {
                    protocol::Version::Rfb33 | protocol::Version::Rfb37 =>
                        return Err(Error::AuthenticationFailure(String::from(""))),
                    protocol::Version::Rfb38 => {
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
                       format: protocol::PixelFormat) ->
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
