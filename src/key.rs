use std::process::exit;

use crate::{
    convert::{key_to_tag, tag_to_key},
    options::TagCommands,
};

pub async fn key_main(command: TagCommands) {
    match command {
        TagCommands::Encode { key } => {
            print!("{}", key_to_tag(&key));
        }
        TagCommands::Decode { tag } => match tag_to_key(&tag) {
            Ok(key) => print!("{}", key),
            Err(e) => {
                eprintln!("error: {}", e);
                exit(1)
            }
        },
    }
}
