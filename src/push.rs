use std::{env, process::exit};

use oci_distribution::secrets::RegistryAuth;

use crate::{
    options::{InitializeOptions, PushOptions, PushSubcommands},
    registry::{self, build_reference},
};

fn get_auth() -> RegistryAuth {
    let username = match env::var("ORANC_USERNAME") {
        Ok(u) => u,
        Err(e) => {
            log::info!(
                "use anonymous auth, invalid environment variable `ORANC_USERNAME`: {}",
                e
            );
            return RegistryAuth::Anonymous;
        }
    };
    let password = match env::var("ORANC_PASSWORD") {
        Ok(u) => u,
        Err(e) => {
            log::info!(
                "use anonymous auth, invalid environment variable `ORANC_PASSWORD`: {}",
                e
            );
            return RegistryAuth::Anonymous;
        }
    };
    RegistryAuth::Basic(username, password)
}

fn build_nix_cache_info(initialize_options: &InitializeOptions) -> String {
    format!(
        "StoreDir: {store_dir}
WantMassQuery: {mass_query}
Priority: {priority}
",
        store_dir = initialize_options.store_dir,
        mass_query = if initialize_options.no_mass_query {
            0
        } else {
            1
        },
        priority = initialize_options.priority
    )
}

pub async fn push_main(options: PushOptions) {
    let auth = get_auth();
    if let Some(cmd) = &options.subcommand {
        match cmd {
            PushSubcommands::Initialize(initialize_options) => {
                push_initialize_main(auth, options.clone(), initialize_options.clone()).await
            }
        }
        exit(0); // just exit after subcommand main
    }
    // real main for push
}

pub async fn push_initialize_main(
    auth: RegistryAuth,
    options: PushOptions,
    initialize_options: InitializeOptions,
) {
    let nix_cache_info = build_nix_cache_info(&initialize_options);
    let key = "nix-cache-info";
    let content_type = "text/x-nix-cache-info";
    let reference = build_reference(options.registry, options.repository, key);
    let mut client = Default::default();
    match registry::put(
        &mut client,
        &reference,
        &auth,
        key,
        Some(content_type.to_string()),
        nix_cache_info.as_bytes().to_vec(),
    )
    .await
    {
        Ok(_response) => print!("{}", nix_cache_info),
        Err(e) => {
            eprintln!("error: {}", e);
            exit(1)
        }
    }
}
