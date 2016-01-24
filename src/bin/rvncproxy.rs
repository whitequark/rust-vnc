extern crate env_logger;
#[macro_use] extern crate log;
#[macro_use] extern crate clap;
extern crate vnc;

use clap::{Arg, App};

fn main() {
    env_logger::init().unwrap();

    let matches = App::new("rvncclient")
        .about("VNC proxy")
        .arg(Arg::with_name("CONNECT-HOST")
                .help("server hostname or IP")
                .required(true)
                .index(1))
        .arg(Arg::with_name("CONNECT-PORT")
                .help("server port (default: 5900)")
                .index(2))
        .arg(Arg::with_name("LISTEN-HOST")
                .help("proxy hostname or IP (default: localhost)")
                .index(3))
        .arg(Arg::with_name("LISTEN-PORT")
                .help("proxy port (default: server port plus one)")
                .index(4))
        .get_matches();

    let connect_host = matches.value_of("CONNECT-HOST")
        .unwrap();
    let connect_port = value_t!(matches.value_of("CONNECT-PORT"), u16)
        .unwrap_or(5900);
    let listen_host = matches.value_of("LISTEN-HOST")
        .unwrap_or("localhost");
    let listen_port = value_t!(matches.value_of("LISTEN-PORT"), u16)
        .unwrap_or(connect_port + 1);

    info!("listening at {}:{}", listen_host, listen_port);
    let listener =
        match std::net::TcpListener::bind((listen_host, listen_port)) {
            Ok(listener) => listener,
            Err(error) => {
                error!("cannot listen at {}:{}: {}", listen_host, listen_port, error);
                std::process::exit(1)
            }
        };

    for incoming_stream in listener.incoming() {
        let client_stream =
            match incoming_stream {
                Ok(stream) => stream,
                Err(error) => {
                    error!("incoming connection failed: {}", error);
                    continue
                }
            };

        info!("connecting to {}:{}", connect_host, connect_port);
        let server_stream =
            match std::net::TcpStream::connect((connect_host, connect_port)) {
                Ok(stream) => stream,
                Err(error) => {
                    error!("cannot connect to {}:{}: {}", connect_host, connect_port, error);
                    client_stream.shutdown(std::net::Shutdown::Both).unwrap();
                    continue
                }
            };

        let proxy =
            match vnc::proxy::Proxy::from_tcp_streams(server_stream, client_stream) {
                Ok(proxy) => proxy,
                Err(error) => {
                    error!("handshake failed: {}", error);
                    continue
                }
            };

        match proxy.join() {
            Ok(()) => info!("session ended"),
            Err(error) => error!("session failed: {}", error)
        }
    }
}
