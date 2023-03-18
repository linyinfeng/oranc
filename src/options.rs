use clap::Parser;
use std::net::SocketAddr;

#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Options {
    #[arg(short, long, default_value = "[::]:8080")]
    pub listen: SocketAddr,
}
