#[macro_use]
extern crate log;
extern crate byteorder;
extern crate flate2;

mod protocol;
mod security;
mod zrle;

pub mod client;
pub mod proxy;

pub use client::Client;
pub use protocol::{Colour, Encoding, PixelFormat};
pub use proxy::Proxy;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Rect {
    pub left: u16,
    pub top: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Unexpected(&'static str),
    Server(String),
    AuthenticationUnavailable,
    AuthenticationFailure(String),
    Disconnected,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::result::Result<(), std::fmt::Error> {
        match self {
            Error::Io(ref inner) => inner.fmt(f),
            Error::Unexpected(ref descr) => write!(f, "unexpected {}", descr),
            Error::Server(ref descr) => write!(f, "server error: {}", descr),
            Error::AuthenticationFailure(ref descr) => {
                write!(f, "authentication failure: {}", descr)
            }
            Error::AuthenticationUnavailable => {
                write!(f, "authentication unavailable")
            }
            Error::Disconnected => {
                write!(f, "disconnected")
            }
        }
    }
}

impl std::error::Error for Error {
    fn cause(&self) -> Option<&dyn std::error::Error> {
        match self {
            Error::Io(ref inner) => Some(inner),
            _ => None,
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Error {
        Error::Io(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
