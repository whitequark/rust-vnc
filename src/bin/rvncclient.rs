extern crate env_logger;
#[macro_use] extern crate log;
#[macro_use] extern crate clap;
extern crate vnc;

use clap::{Arg, App};

fn authenticate(methods: &[vnc::AuthMethod]) -> Option<vnc::AuthChoice> {
    for method in methods {
        match method {
            &vnc::AuthMethod::None => return Some(vnc::AuthChoice::None),
            _ => ()
        }
    }
    None
}

fn connect(host: &str, port: u16) -> vnc::Result<()> {
    info!("connecting to {}:{}", host, port);
    let stream = try!(std::net::TcpStream::connect((host, port)));
    let vnc = try!(vnc::Client::from_tcp_stream(stream, authenticate, false));
    Ok(())
}

fn main() {
    env_logger::init().unwrap();

    let matches = App::new("rvncclient")
        .about("VNC client")
        .arg(Arg::with_name("HOST")
                .help("VNC server hostname or IP")
                .required(true)
                .index(1))
        .arg(Arg::with_name("PORT")
                .help("VNC server port")
                .index(2))
        .get_matches();

    let host = matches.value_of("HOST").unwrap();
    let port = value_t!(matches.value_of("PORT"), u16).unwrap_or(5900);

    connect(host, port).unwrap();
}
