mod des;
pub use self::des::encrypt as des;

#[cfg(feature = "apple-auth")]
mod apple;
#[cfg(feature = "apple-auth")]
mod md5;
#[cfg(feature = "apple-auth")]
pub use self::apple::apple_auth;
