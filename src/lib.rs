#[macro_use] extern crate log;
// TODO: https://github.com/BurntSushi/byteorder/pull/40
extern crate byteorder;

mod protocol;
pub mod client;
pub mod proxy;

pub use protocol::{Version, PixelFormat};

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Rect {
    pub left:   u16,
    pub top:    u16,
    pub width:  u16,
    pub height: u16
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    FromUtf8(std::string::FromUtf8Error),

    UnexpectedEOF,
    UnexpectedValue(&'static str),
    Disconnected,
    AuthenticationUnavailable,
    AuthenticationFailure(String),
    Server(String)
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::result::Result<(), std::fmt::Error> {
        match self {
            &Error::Io(ref inner) => inner.fmt(f),
            &Error::FromUtf8(ref inner) => inner.fmt(f),
            &Error::UnexpectedValue(ref descr) =>
                write!(f, "unexpected value for {}", descr),
            &Error::AuthenticationFailure(ref descr) =>
                write!(f, "authentication failure: {}", descr),
            &Error::Server(ref descr) =>
                write!(f, "server error: {}", descr),
            _ => f.write_str(std::error::Error::description(self))
        }
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        match self {
            &Error::Io(ref inner) => inner.description(),
            &Error::FromUtf8(ref inner) => inner.description(),
            &Error::UnexpectedEOF => "unexpected EOF",
            &Error::UnexpectedValue(_) => "unexpected value",
            &Error::Disconnected => "graceful disconnect",
            &Error::AuthenticationUnavailable => "authentication unavailable",
            &Error::AuthenticationFailure(_) => "authentication failure",
            &Error::Server(_) => "server error",
        }
    }

    fn cause(&self) -> Option<&std::error::Error> {
        match self {
            &Error::Io(ref inner) => Some(inner),
            &Error::FromUtf8(ref inner) => Some(inner),
            _ => None
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Error { Error::Io(error) }
}

impl From<std::string::FromUtf8Error> for Error {
    fn from(error: std::string::FromUtf8Error) -> Error { Error::FromUtf8(error) }
}

impl From<byteorder::Error> for Error {
    fn from(error: byteorder::Error) -> Error {
        match error {
            byteorder::Error::UnexpectedEOF => Error::UnexpectedEOF,
            byteorder::Error::Io(inner) => Error::Io(inner)
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
