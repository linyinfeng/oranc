use crate::{error::Error, options::TagCommands};

pub async fn key_main(command: TagCommands) -> Result<(), Error> {
    match command {
        TagCommands::Encode {
            key,
            fallbacks,
            encoding_options,
        } => {
            let (m, f) = encoding_options.key_to_tag(&key);
            println!("{}", m);
            if fallbacks {
                for tag in f {
                    println!("{}", tag);
                }
            }
        }
        TagCommands::Decode {
            tag,
            encoding_options,
        } => println!("{}", encoding_options.tag_to_key(&tag)?),
    }
    Ok(())
}
