use maplit::hashmap;
use oci_distribution::{
    client::{Config, ImageLayer, PushResponse},
    config::{Architecture, ConfigFile, Os, Rootfs},
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
) -> Result<Option<LayerInfo>, Error> {
    let (manifest, _hash) = match client.pull_image_manifest(reference, auth).await {
        Ok(t) => t,
        Err(e) => {
            log::trace!("failed to get layer info: {e}");
            return Ok(None);
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
    client: &mut Client,
    reference: &Reference,
    auth: &RegistryAuth,
    key: &str,
    optional_content_type: Option<String>,
    data: Vec<u8>,
) -> Result<PushResponse, Error> {
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
    client
        .push(reference, &layers, config, auth, Some(image_manifest))
        .await
        .map_err(Error::OciDistribution)
}
