use maplit::hashmap;
use oci_distribution::{
    client::{Config, ImageLayer},
    config::{Architecture, ConfigFile, Os, Rootfs},
    errors::{OciDistributionError, OciErrorCode},
    manifest::OciImageManifest,
    secrets::RegistryAuth,
    Client, Reference,
};

use crate::{convert::key_to_tag, error::Error};

pub const LAYER_MEDIA_TYPE: &str = "application/octet-stream";
pub const CONTENT_TYPE_ANNOTATION: &str = "com.linyinfeng.oranc.content.type";

#[derive(Debug, Clone)]
pub struct LayerInfo {
    pub digest: String,
    pub content_type: String,
}

pub fn build_reference(registry: String, repository: String, key: &str) -> Reference {
    Reference::with_tag(registry, repository, key_to_tag(key))
}

pub async fn get_layer_info(
    client: &mut Client,
    reference: &Reference,
    auth: &RegistryAuth,
    max_retry: usize,
) -> Result<Option<LayerInfo>, Error> {
    if max_retry < 1 {
        return Err(Error::InvalidMaxRetry(max_retry));
    }
    let mut pull_result = None;
    let mut errors = vec![];
    for attempt in 1..max_retry {
        log::debug!("pull image manifest {reference:?}, attempt {attempt}/{max_retry}");
        match client.pull_image_manifest(reference, auth).await {
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

// TODO refactor server and simplify put function
#[allow(clippy::too_many_arguments)]
pub async fn put(
    client: &mut Client,
    reference: &Reference,
    auth: &RegistryAuth,
    key: &str,
    optional_content_type: Option<String>,
    data: Vec<u8>,
    max_retry: usize,
    dry_run: bool,
) -> Result<(), Error> {
    let content_type = match optional_content_type {
        None => "application/octet-stream".to_string(),
        Some(c) => c,
    };
    let layer_annotations = hashmap! {
        CONTENT_TYPE_ANNOTATION.to_string() => content_type,
    };
    let layer = ImageLayer::new(data, LAYER_MEDIA_TYPE.to_string(), Some(layer_annotations));
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
        history: vec![],
    };
    let config_annotations = None;
    let config = Config::oci_v1_from_config_file(config_file, config_annotations)
        .map_err(Error::OciDistribution)?;

    let image_annotations = hashmap! {
        "com.linyinfeng.oranc.key".to_string() => key.to_owned(),
        "org.opencontainers.image.description".to_string() => key.to_owned(),
    };
    let image_manifest = OciImageManifest::build(&layers, &config, Some(image_annotations));

    if max_retry < 1 {
        return Err(Error::InvalidMaxRetry(max_retry));
    }
    let mut errors = vec![];
    for attempt in 1..max_retry {
        log::debug!("push {reference:?}, attempt {attempt}/{max_retry}");
        if dry_run {
            log::debug!("dry run, skipped");
            return Ok(());
        }
        match client
            .push(
                reference,
                &layers,
                config.clone(),
                auth,
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
