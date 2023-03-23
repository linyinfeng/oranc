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

#[tokio::main]
async fn main() {
    pretty_env_logger::init();

    let options = options::Options::parse();
    log::debug!("options = {:#?}", options);
    if let Err(e) = main_result(options).await {
        log::error!("{}", e);
        std::process::exit(1);
    }
}

async fn main_result(options: Options) -> Result<(), Error> {
    match options.command {
        Commands::Server(server_options) => server::server_main(server_options).await?,
        Commands::Tag(key_commands) => key::key_main(key_commands).await?,
        Commands::Push(push_options) => push::push_main(push_options).await?,
    }
    Ok(())
}
