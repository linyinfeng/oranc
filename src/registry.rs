use std::fmt;

use maplit::hashmap;
use oci_distribution::{
    client::{ClientConfig, ClientProtocol, Config, ImageLayer},
    config::{Architecture, ConfigFile, Os, Rootfs},
    errors::{OciDistributionError, OciErrorCode},
    manifest::OciImageManifest,
    secrets::RegistryAuth,
    Client, Reference,
};

use crate::{
    convert::key_to_tag,
    error::Error,
    options::{PushOptions, ServerOptions},
};

pub const LAYER_MEDIA_TYPE: &str = "application/octet-stream";
pub const CONTENT_TYPE_ANNOTATION: &str = "com.linyinfeng.oranc.content.type";

pub struct RegistryContext {
    pub options: RegistryOptions,
    pub client: Client,
    pub auth: RegistryAuth,
}

#[derive(Debug, Clone)]
pub struct RegistryOptions {
    pub no_ssl: bool,
    pub dry_run: bool,
    pub max_retry: usize,
}

#[derive(Debug, Clone)]
pub struct OciLocation {
    pub registry: String,
    pub repository: String,
    pub key: String,
}

#[derive(Debug, Clone)]
pub struct OciItem {
    pub content_type: Option<String>,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct LayerInfo {
    pub digest: String,
    pub content_type: String,
}

impl OciLocation {
    pub fn reference(&self) -> Reference {
        Reference::with_tag(
            self.registry.clone(),
            self.repository.clone(),
            key_to_tag(&self.key),
        )
    }
}

pub async fn get_layer_info(
    ctx: &mut RegistryContext,
    location: &OciLocation,
) -> Result<Option<LayerInfo>, Error> {
    let max_retry = ctx.options.max_retry;
    if max_retry < 1 {
        return Err(Error::InvalidMaxRetry(max_retry));
    }

    let reference = location.reference();
    let mut pull_result = None;
    let mut errors = vec![];
    for attempt in 1..max_retry {
        log::debug!("pull image manifest {reference:?}, attempt {attempt}/{max_retry}");
        match ctx.client.pull_image_manifest(&reference, &ctx.auth).await {
            Ok(r) => pull_result = Some(r),
            Err(OciDistributionError::ImageManifestNotFoundError(_)) => return Ok(None),
            Err(OciDistributionError::RegistryError { envelope, .. })
                if envelope
                    .errors
                    .iter()
                    .all(|e| e.code == OciErrorCode::ManifestUnknown) =>
            {
                return Ok(None)
            }
            Err(oci_error) => {
                let e = oci_error.into();
                log::warn!(
                    "pull image manifest {reference:?}, attempt {attempt}/{max_retry} failed: {}",
                    e
                );
                errors.push(e);
            }
        }
    }
    let (manifest, _hash) = match pull_result {
        Some(r) => r,
        None => return Err(Error::RetryAllFails(errors)),
    };

    match manifest.layers.len() {
        1 => (),
        other => return Err(Error::InvalidLayerCount(other)),
    }
    let layer_manifest = &manifest.layers[0];
    if layer_manifest.media_type != LAYER_MEDIA_TYPE {
        return Err(Error::InvalidLayerMediaType(
            layer_manifest.media_type.clone(),
        ));
    }
    let annotations = match &layer_manifest.annotations {
        Some(a) => a,
        None => return Err(Error::NoLayerAnnotations),
    };
    let content_type = match annotations.get(CONTENT_TYPE_ANNOTATION) {
        Some(a) => a,
        None => {
            return Err(Error::NoLayerAnnotationKey(
                CONTENT_TYPE_ANNOTATION.to_string(),
            ))
        }
    };
    let info = LayerInfo {
        digest: layer_manifest.digest.clone(),
        content_type: content_type.clone(),
    };
    Ok(Some(info))
}

pub async fn put(
    ctx: &mut RegistryContext,
    location: &OciLocation,
    oci_item: OciItem,
) -> Result<(), Error> {
    let content_type = match oci_item.content_type {
        None => "application/octet-stream".to_string(),
        Some(c) => c,
    };
    let layer_annotations = hashmap! {
        CONTENT_TYPE_ANNOTATION.to_string() => content_type,
    };
    let layer = ImageLayer::new(
        oci_item.data,
        LAYER_MEDIA_TYPE.to_string(),
        Some(layer_annotations),
    );
    let layer_digest = layer.sha256_digest();
    let layers = vec![layer];

    let rootfs = Rootfs {
        r#type: "layers".to_string(),
        diff_ids: vec![
            // just use layer digest
            layer_digest,
        ],
    };
    let config_file = ConfigFile {
        created: None,
        author: None,
        architecture: Architecture::None,
        os: Os::None,
        config: None,
        rootfs,
        history: None,
    };
    let config_annotations = None;
    let config = Config::oci_v1_from_config_file(config_file, config_annotations)
        .map_err(Error::OciDistribution)?;

    let key = &location.key;
    let image_annotations = hashmap! {
        "com.linyinfeng.oranc.key".to_string() => key.to_owned(),
        "org.opencontainers.image.description".to_string() => key.to_owned(),
    };
    let image_manifest = OciImageManifest::build(&layers, &config, Some(image_annotations));

    let max_retry = ctx.options.max_retry;
    if max_retry < 1 {
        return Err(Error::InvalidMaxRetry(max_retry));
    }
    let reference = location.reference();
    let mut errors = vec![];
    for attempt in 1..max_retry {
        log::debug!("push {reference:?}, attempt {attempt}/{max_retry}");
        if ctx.options.dry_run {
            log::debug!("dry run, skipped");
            return Ok(());
        }
        match ctx
            .client
            .push(
                &reference,
                &layers,
                config.clone(),
                &ctx.auth,
                Some(image_manifest.clone()),
            )
            .await
        {
            Ok(_push_response) => return Ok(()),
            Err(oci_error) => {
                let e = oci_error.into();
                log::warn!(
                    "push {reference:?}, attempt {attempt}/{max_retry} failed: {}",
                    e
                );
                errors.push(e);
            }
        }
    }
    Err(Error::RetryAllFails(errors))
}

impl RegistryOptions {
    pub fn from_push_options(options: &PushOptions) -> Self {
        Self {
            dry_run: options.dry_run,
            max_retry: options.max_retry,
            no_ssl: options.no_ssl,
        }
    }

    pub fn from_server_options(options: &ServerOptions) -> Self {
        Self {
            dry_run: false,
            max_retry: options.max_retry,
            no_ssl: options.no_ssl,
        }
    }

    pub fn client_config(&self) -> ClientConfig {
        let protocol = if self.no_ssl {
            ClientProtocol::Http
        } else {
            ClientProtocol::Https
        };
        ClientConfig {
            protocol,
            ..Default::default()
        }
    }

    pub fn client(&self) -> Client {
        Client::new(self.client_config())
    }

    pub fn context(self, auth: RegistryAuth) -> RegistryContext {
        let client = self.client();
        RegistryContext {
            options: self,
            client,
            auth,
        }
    }
}

impl fmt::Display for OciLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}/{}", self.registry, self.repository, self.key)
    }
}
