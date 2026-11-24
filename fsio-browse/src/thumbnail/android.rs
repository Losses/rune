//! Android document providers draw thumbnails for the documents they serve,
//! the same ones the system's Files app shows.

use anyhow::Result;
use jni::objects::{JObject, JValue};

use super::{Thumbnail, ThumbnailSize, fit_pixels, unpremultiply};
use crate::android_jni::{content_resolver, with_env};

pub(super) fn thumbnail(uri: &str, size: ThumbnailSize) -> Result<Option<Thumbnail>> {
    let ThumbnailSize(edge) = size;
    let edge = i32::try_from(edge).unwrap_or(i32::MAX);
    let pixels = with_env(|env| {
        let resolver = content_resolver(env)?;
        let text = env.new_string(uri)?;
        let uri = env
            .call_static_method(
                "android/net/Uri",
                "parse",
                "(Ljava/lang/String;)Landroid/net/Uri;",
                &[JValue::Object(&text)],
            )?
            .l()?;
        let point = env.new_object(
            "android/graphics/Point",
            "(II)V",
            &[JValue::Int(edge), JValue::Int(edge)],
        )?;
        // A provider without thumbnails for this document throws; that is
        // the provider's answer, and the item keeps its type icon.
        let bitmap = match env.call_static_method(
            "android/provider/DocumentsContract",
            "getDocumentThumbnail",
            "(Landroid/content/ContentResolver;Landroid/net/Uri;Landroid/graphics/Point;Landroid/os/CancellationSignal;)Landroid/graphics/Bitmap;",
            &[
                JValue::Object(&resolver),
                JValue::Object(&uri),
                JValue::Object(&point),
                JValue::Object(&JObject::null()),
            ],
        ) {
            Ok(value) => value.l()?,
            Err(_) => {
                env.exception_clear()?;
                return Ok(None);
            }
        };
        if bitmap.is_null() {
            return Ok(None);
        }
        // Pixels are copied out as ARGB_8888, which lies in memory as
        // premultiplied RGBA.
        let config = env
            .get_static_field(
                "android/graphics/Bitmap$Config",
                "ARGB_8888",
                "Landroid/graphics/Bitmap$Config;",
            )?
            .l()?;
        let bitmap = env
            .call_method(
                &bitmap,
                "copy",
                "(Landroid/graphics/Bitmap$Config;Z)Landroid/graphics/Bitmap;",
                &[JValue::Object(&config), JValue::Bool(0)],
            )?
            .l()?;
        let width = env.call_method(&bitmap, "getWidth", "()I", &[])?.i()?;
        let height = env.call_method(&bitmap, "getHeight", "()I", &[])?.i()?;
        let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
            return Ok(None);
        };
        let mut pixels = vec![0u8; width as usize * height as usize * 4];
        // SAFETY: the buffer outlives the Java object, which is used only
        // for the copy below.
        let buffer = unsafe { env.new_direct_byte_buffer(pixels.as_mut_ptr(), pixels.len())? };
        env.call_method(
            &bitmap,
            "copyPixelsToBuffer",
            "(Ljava/nio/Buffer;)V",
            &[JValue::Object(&buffer)],
        )?;
        env.call_method(&bitmap, "recycle", "()V", &[])?;
        Ok(Some((width, height, pixels)))
    })?;
    let Some((width, height, mut rgba)) = pixels else {
        return Ok(None);
    };
    unpremultiply(&mut rgba, 3);
    Ok(fit_pixels(width, height, rgba, size))
}
