use clap::{Parser, Subcommand};
use regex::Regex;
use reqwest::Url;
use std::net::SocketAddr;

#[derive(Clone, Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Options {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Clone, Debug, Subcommand)]
pub enum Commands {
    Server(ServerOptions),
    #[command(subcommand)]
    Tag(TagCommands),
}

#[derive(Clone, Debug, Parser)]
#[command(about = "HTTP Nix cache server backed by OCI Registry")]
pub struct ServerOptions {
    #[arg(short, long, default_value = "[::]:8080")]
    pub listen: SocketAddr,
    #[arg(short, long, value_name = "URL")]
    pub upstream: Vec<Url>,
    #[arg(short, long, value_name = "PATTERN", default_value = "nix-cache-info")]
    pub ignore_upstream: Regex,
    #[arg(long)]
    pub upstream_anonymous: bool,
}

#[derive(Clone, Debug, Subcommand)]
#[command(about = "Command line tools for tag-key conversion")]
pub enum TagCommands {
    #[command(about = "Encode a key to tag")]
    Encode { key: String },
    #[command(about = "Decode a tag to key")]
    Decode { tag: String },
}
