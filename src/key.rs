use crate::{
    convert::{key_to_tag, tag_to_key},
    error::Error,
    options::TagCommands,
};

pub async fn key_main(command: TagCommands) -> Result<(), Error> {
    match command {
        TagCommands::Encode { key } => print!("{}", key_to_tag(&key)),
        TagCommands::Decode { tag } => print!("{}", tag_to_key(&tag)?),
    }
    Ok(())
}
