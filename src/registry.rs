use std::fmt;

use crate::convert::EncodingOptions;
use crate::{
    error::Error,
    options::{PushOptions, ServerOptions},
};
use maplit::hashmap;
use oci_distribution::{
    client::{ClientConfig, ClientProtocol, Config, ImageLayer},
    config::{Architecture, ConfigFile, Os, Rootfs},
    errors::{OciDistributionError, OciErrorCode},
    manifest::OciImageManifest,
    secrets::RegistryAuth,
    Client, Reference,
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
    pub encoding_options: EncodingOptions,
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
    pub reference: Reference,
    pub digest: String,
    pub content_type: String,
}

impl OciLocation {
    pub fn reference(&self, encoding_options: &EncodingOptions) -> (Reference, Vec<Reference>) {
        let build_ref =
            |tag| Reference::with_tag(self.registry.clone(), self.repository.clone(), tag);
        let (main_tag, fallback_tags) = encoding_options.key_to_tag(&self.key);
        let main = build_ref(main_tag);
        let fallback_refs = fallback_tags.into_iter().map(build_ref).collect();
        (main, fallback_refs)
    }

    pub fn references_merged(&self, encoding_options: &EncodingOptions) -> Vec<Reference> {
        let (main, fallbacks) = self.reference(encoding_options);
        let mut result = vec![main];
        result.extend(fallbacks);
        result
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

    let references = location.references_merged(&ctx.options.encoding_options);
    let mut pull_result = None;
    let mut errors = vec![];
    'fallbacks: for reference in references {
        let mut ref_errors = vec![];
        'retries: for attempt in 1..max_retry {
            log::debug!("pull image manifest {reference:?}, attempt {attempt}/{max_retry}");
            match ctx.client.pull_image_manifest(&reference, &ctx.auth).await {
                Ok(res) => {
                    pull_result = Some((reference.clone(), res));
                    break 'fallbacks;
                }
                Err(OciDistributionError::ImageManifestNotFoundError(_)) => break 'retries,
                Err(OciDistributionError::RegistryError { envelope, .. })
                    if envelope
                        .errors
                        .iter()
                        .all(|e| e.code == OciErrorCode::ManifestUnknown) =>
                {
                    break 'retries;
                }
                Err(oci_error) => {
                    let e = oci_error.into();
                    log::warn!(
                        "pull image manifest {reference:?}, attempt {attempt}/{max_retry} failed: {}",
                        e
                    );
                    ref_errors.push(e);
                }
            }
        }
        if ref_errors.len() == max_retry {
            log::error!("pull image manifest {reference:?} failed");
            // all reties failed
            errors.extend(ref_errors);
        }
    }
    let (reference, (manifest, _hash)) = match pull_result {
        Some(r) => r,
        None => {
            if errors.is_empty() {
                // all reference not found
                return Ok(None);
            } else {
                // at least one reference failed
                return Err(Error::RetryAllFails(errors));
            }
        }
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
            ));
        }
    };
    let info = LayerInfo {
        reference,
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
    let (reference, _fallbacks) = location.reference(&ctx.options.encoding_options);
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
            encoding_options: options.encoding_options.clone(),
        }
    }

    pub fn from_server_options(options: &ServerOptions) -> Self {
        Self {
            dry_run: false,
            max_retry: options.max_retry,
            no_ssl: options.no_ssl,
            encoding_options: options.encoding_options.clone(),
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
