#[macro_use] extern crate log;
extern crate byteorder;
extern crate flate2;
#[cfg(feature = "apple-auth")]
extern crate num_bigint;
#[cfg(feature = "apple-auth")]
extern crate octavo;
#[cfg(feature = "apple-auth")]
extern crate crypto;

mod protocol;
mod zrle;
mod security;

pub mod client;
pub mod proxy;
pub mod server;

pub use protocol::{PixelFormat, Colour, Encoding};
pub use client::Client;
pub use proxy::Proxy;
pub use server::Server;

pub mod pixel_format {
    use super::PixelFormat;

    /// RGB pixel format with 4 bytes per pixel and 3 bytes of depth.
    pub const RGB8888: PixelFormat = PixelFormat {
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
    };

    /// BGR pixel format with 4 bytes per pixel and 3 bytes of depth.
    pub const BGR8888: PixelFormat = PixelFormat {
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
    };
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Unexpected(&'static str),
    Server(String),
    AuthenticationUnavailable,
    AuthenticationFailure(String),
    Disconnected
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::result::Result<(), std::fmt::Error> {
        match self {
            &Error::Io(ref inner) => inner.fmt(f),
            &Error::Unexpected(ref descr) =>
                write!(f, "unexpected {}", descr),
            &Error::Server(ref descr) =>
                write!(f, "server error: {}", descr),
            &Error::AuthenticationFailure(ref descr) =>
                write!(f, "authentication failure: {}", descr),
            _ => f.write_str(std::error::Error::description(self))
        }
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        match self {
            &Error::Io(ref inner) => inner.description(),
            &Error::Unexpected(_) => "unexpected value",
            &Error::Server(_) => "server error",
            &Error::AuthenticationUnavailable => "authentication unavailable",
            &Error::AuthenticationFailure(_) => "authentication failure",
            &Error::Disconnected => "peer disconnected",
        }
    }

    fn cause(&self) -> Option<&std::error::Error> {
        match self {
            &Error::Io(ref inner) => Some(inner),
            _ => None
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Error { Error::Io(error) }
}

pub type Result<T> = std::result::Result<T, Error>;
