#![feature(read_exact)]

#[macro_use] extern crate log;
extern crate byteorder;

mod protocol;

pub use protocol::{Error, Result, Version, PixelFormat};

use std::net::TcpStream;
use protocol::Message;

pub enum AuthMethod {
    None,
    /* more to come */
    Unused
}

pub enum AuthChoice {
    None,
    /* more to come */
}

pub struct Client {
    socket:  TcpStream,
    version: Version,
    name:    String,
    size:    (u16, u16),
    format:  protocol::PixelFormat,
}

impl Client {
    pub fn from_tcp_stream<Auth>(mut socket: TcpStream, auth: Auth, shared: bool) -> Result<Client>
            where Auth: FnOnce(&[AuthMethod]) -> Option<AuthChoice> {
        let version = try!(protocol::Version::read_from(&mut socket));
        debug!("<- Version::{:?}", version);
        debug!("-> Version::{:?}", version);
        try!(protocol::Version::write_to(&version, &mut socket));

        let security_types = match version {
            Version::Rfb33 => {
                let security_type = try!(protocol::SecurityType::read_from(&mut socket));
                debug!("<- SecurityType::{:?}", security_type);
                if security_type == protocol::SecurityType::Invalid {
                    let reason = try!(String::read_from(&mut socket));
                    debug!("<- {:?}", reason);
                    return Err(Error::Server(reason))
                }
                vec![security_type]
            },
            _ => {
                let security_types = try!(protocol::SecurityTypes::read_from(&mut socket));
                debug!("<- {:?}", security_types);
                if security_types.0.len() == 0 {
                    let reason = try!(String::read_from(&mut socket));
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
                try!(protocol::SecurityType::write_to(&used_security_type, &mut socket));
            }
        }

        let mut skip_security_result = false;
        match &(auth_choice, version) {
            &(AuthChoice::None, Version::Rfb33) |
            &(AuthChoice::None, Version::Rfb37) => skip_security_result = true,
            _ => ()
        }

        if !skip_security_result {
            let security_result = try!(protocol::SecurityResult::read_from(&mut socket));
            if security_result == protocol::SecurityResult::Failed {
                match version {
                    Version::Rfb33 | Version::Rfb37 =>
                        return Err(Error::AuthenticationFailure(String::from(""))),
                    Version::Rfb38 => {
                        let reason = try!(String::read_from(&mut socket));
                        debug!("<- {:?}", reason);
                        return Err(Error::AuthenticationFailure(reason))
                    }
                }
            }
        }

        let client_init = protocol::ClientInit { shared: shared };
        debug!("-> {:?}", client_init);
        try!(protocol::ClientInit::write_to(&client_init, &mut socket));

        let server_init = try!(protocol::ServerInit::read_from(&mut socket));
        debug!("<- {:?}", server_init);

        let set_encodings = protocol::C2S::SetEncodings(vec![
            protocol::Encoding::Raw,
            protocol::Encoding::DesktopSize
        ]);
        debug!("-> {:?}", set_encodings);
        try!(protocol::C2S::write_to(&set_encodings, &mut socket));

        Ok(Client {
            socket:  socket,
            version: version,
            name:    server_init.name,
            size:    (server_init.framebuffer_width, server_init.framebuffer_height),
            format:  server_init.pixel_format,
        })
    }

    pub fn version(&self) -> Version { self.version }
    pub fn name(&self) -> &str { &self.name }
    pub fn size(&self) -> (u16, u16) { self.size }
}
