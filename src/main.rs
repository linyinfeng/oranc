pub mod convert;
pub mod error;
pub mod key;
pub mod nix;
pub mod options;
pub mod push;
pub mod registry;
pub mod server;

use clap::Parser;

use error::Error;
use options::{Commands, Options};
use pretty_env_logger::formatted_builder;

#[tokio::main]
async fn main() {
    init_logger();

    let options = options::Options::parse();
    log::debug!("options = {:#?}", options);
    if let Err(e) = main_result(options).await {
        log::error!("{}", e);
        std::process::exit(1);
    }
}

fn init_logger() {
    let mut builder = formatted_builder();
    let filters = match std::env::var("RUST_LOG") {
        Ok(f) => f,
        Err(_) => "oranc=info".to_string(),
    };
    builder.parse_filters(&filters);
    builder.init()
}

async fn main_result(options: Options) -> Result<(), Error> {
    match options.command {
        Commands::Server(server_options) => server::server_main(server_options).await?,
        Commands::Tag(key_commands) => key::key_main(key_commands).await?,
        Commands::Push(push_options) => push::push_main(push_options).await?,
    }
    Ok(())
}
