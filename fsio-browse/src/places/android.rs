//! Android apps reach shared storage only through folders the person granted
//! in the system's document picker. Those grants are the device's volumes
//! here; there are no pinned or well-known folders to read.

use std::path::PathBuf;

use anyhow::Result;
use jni::objects::{JString, JValue};

use super::{KnownFolder, PinnedFolder, Volume, VolumeKind};
use crate::android_jni::{content_resolver, with_env};
use crate::location::{Location, Root, percent_decode};

pub(super) fn known_folders() -> Vec<(KnownFolder, PathBuf)> {
    Vec::new()
}

pub(super) fn pinned_folders() -> Vec<PinnedFolder> {
    Vec::new()
}

pub(super) fn volumes() -> Vec<Volume> {
    match granted_trees() {
        Ok(uris) => uris.into_iter().map(|uri| tree_volume(&uri)).collect(),
        Err(error) => {
            log::warn!("Could not list granted folders: {error:#}");
            Vec::new()
        }
    }
}

/// The tree URIs Rune holds a lasting read grant for.
fn granted_trees() -> Result<Vec<String>> {
    let uris = with_env(|env| {
        let resolver = content_resolver(env)?;
        let grants = env
            .call_method(
                &resolver,
                "getPersistedUriPermissions",
                "()Ljava/util/List;",
                &[],
            )?
            .l()?;
        let count = env.call_method(&grants, "size", "()I", &[])?.i()?;
        let mut uris = Vec::new();
        for index in 0..count {
            let grant = env
                .call_method(
                    &grants,
                    "get",
                    "(I)Ljava/lang/Object;",
                    &[JValue::Int(index)],
                )?
                .l()?;
            let readable = env
                .call_method(&grant, "isReadPermission", "()Z", &[])?
                .z()?;
            let uri = env
                .call_method(&grant, "getUri", "()Landroid/net/Uri;", &[])?
                .l()?;
            let text = env
                .call_method(&uri, "toString", "()Ljava/lang/String;", &[])?
                .l()?;
            let text: String = env.get_string(&JString::from(text))?.into();
            if readable {
                uris.push(text);
            }
            for local in [grant, uri] {
                env.delete_local_ref(local)?;
            }
        }
        Ok(uris)
    })?;
    Ok(uris
        .into_iter()
        .filter(|uri| uri.contains("/tree/"))
        .collect())
}

/// A granted tree as a volume: named after the folder it grants, internal
/// storage when it lies on the primary volume and removable otherwise.
fn tree_volume(uri: &str) -> Volume {
    let document = uri
        .rsplit_once("/tree/")
        .map(|(_, id)| id.split('/').next().unwrap_or(id))
        .and_then(percent_decode)
        .unwrap_or_default();
    let (volume, folder) = document.split_once(':').unwrap_or((&document, ""));
    let label = folder
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .map(str::to_owned);
    Volume {
        location: Location {
            root: Root::DocumentTree {
                uri: uri.to_owned(),
            },
            path: PathBuf::new(),
        },
        label,
        kind: if volume == "primary" {
            VolumeKind::Fixed
        } else {
            VolumeKind::Removable
        },
        space: None,
    }
}
