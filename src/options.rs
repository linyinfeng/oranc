use clap::Parser;
use regex::Regex;
use reqwest::Url;
use std::net::SocketAddr;

#[derive(Clone, Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Options {
    #[arg(short, long, default_value = "[::]:8080")]
    pub listen: SocketAddr,
    #[arg(short, long, value_name = "URL")]
    pub upstream: Vec<Url>,
    #[arg(short, long, value_name = "PATTERN", default_value = "nix-cache-info")]
    pub ignore_upstream: Regex,
}
