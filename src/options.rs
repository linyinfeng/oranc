use clap::{Parser, Subcommand};
use regex::Regex;

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
    Push(PushOptions),
}

#[derive(Clone, Debug, Parser)]
#[command(about = "HTTP Nix cache server backed by OCI Registry")]
pub struct ServerOptions {
    #[arg(short, long, default_value = "[::]:8080")]
    pub listen: SocketAddr,
    #[arg(short, long, value_name = "NUM", default_value = "3")]
    pub max_retry: usize,
}

#[derive(Clone, Debug, Subcommand)]
#[command(about = "Command line tools for tag-key conversion")]
pub enum TagCommands {
    #[command(about = "Encode a key to tag")]
    Encode { key: String },
    #[command(about = "Decode a tag to key")]
    Decode { tag: String },
}

#[derive(Clone, Debug, Parser)]
#[command(about = "Push store paths to OCI Registry")]
pub struct PushOptions {
    #[arg(long, default_value = "ghcr.io")]
    pub registry: String,
    #[arg(long)]
    pub repository: String,
    #[arg(long, help = "do not compute closure for input paths")]
    pub no_closure: bool,
    #[arg(long, default_value = "/nix/store")]
    pub store_dir: String,
    #[arg(
        short,
        long,
        value_name = "REGEX",
        default_value = "^cache\\.nixos\\.org-.*$"
    )]
    pub excluded_signing_key_pattern: Regex,
    #[arg(long, help = "push paths already signed by signing key")]
    pub already_signed: bool,
    #[arg(short, long, value_name = "NUM", default_value = "4")]
    pub parallel: usize,
    #[arg(short, long, value_name = "NUM", default_value = "3")]
    pub zstd_level: i32,
    #[arg(short, long, value_name = "NUM", default_value = "3")]
    pub max_retry: usize,
    #[arg(long)]
    pub dry_run: bool,
    #[arg(long, help = "allow open nix store sqlite database in immutable mode")]
    pub allow_immutable_db: bool,
    #[command(subcommand)]
    pub subcommand: Option<PushSubcommands>,
}

#[derive(Clone, Debug, Subcommand)]
pub enum PushSubcommands {
    Initialize(InitializeOptions),
}

#[derive(Clone, Debug, Parser)]
#[command(about = "Initialize nix-cache-info")]
pub struct InitializeOptions {
    #[arg(short, long, default_value = "41")]
    pub priority: u32,
    #[arg(short, long)]
    pub no_mass_query: bool,
}
