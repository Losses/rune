//! Owned Rust bindings to the statically linked TagLib stream bridge.

use std::{
    collections::BTreeMap,
    ffi::{CStr, CString, c_char, c_int, c_void},
    io::{self, Read, Seek, SeekFrom},
    path::Path,
    slice,
};

use anyhow::{Context, Result, ensure};
use fsio::{FsIo, OpenMode};

#[repr(C)]
struct Callbacks {
    read: unsafe extern "C" fn(*mut c_void, *mut u8, usize) -> usize,
    seek: unsafe extern "C" fn(*mut c_void, i64, c_int) -> i64,
    tag: Option<unsafe extern "C" fn(*mut c_void, *const c_char, *const u8, usize)>,
    picture: Option<
        unsafe extern "C" fn(
            *mut c_void,
            *const c_char,
            *const c_char,
            *const c_char,
            *const u8,
            usize,
        ),
    >,
}

#[repr(C)]
#[derive(Default)]
struct AudioProperties {
    frames: u64,
    duration_ms: i32,
    sample_rate: i32,
    channels: i32,
    bitrate_kbps: i32,
    bits_per_sample: i32,
    format: *const c_char,
    codec: *const c_char,
}

unsafe extern "C" {
    fn rune_taglib_read(
        context: *mut c_void,
        name: *const c_char,
        length: i64,
        callbacks: *const Callbacks,
        properties: *mut AudioProperties,
    ) -> c_int;
}

/// An embedded picture and its native TagLib metadata.
#[derive(Debug)]
pub struct Picture {
    pub mime: String,
    pub description: String,
    pub kind: String,
    pub data: Vec<u8>,
}

/// Metadata read from one audio stream.
#[derive(Debug, Default)]
pub struct Snapshot {
    pub tags: BTreeMap<String, Vec<String>>,
    pub pictures: Vec<Picture>,
    pub format: Option<String>,
    pub codec: Option<String>,

    // Values derived from the actual audio stream.
    pub duration_ms: Option<u64>,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub bitrate: Option<u64>,
    pub bits_per_sample: Option<u32>,
    pub frame_count: Option<u64>,
}

struct Reader<R> {
    source: R,
    snapshot: Snapshot,
    error: Option<io::Error>,
}

impl<R: Read + Seek> Reader<R> {
    unsafe extern "C" fn read(context: *mut c_void, bytes: *mut u8, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        // SAFETY: The bridge passes a live Reader and a writable buffer of len bytes.
        let (reader, mut buffer) = unsafe {
            (
                &mut *context.cast::<Self>(),
                slice::from_raw_parts_mut(bytes, len),
            )
        };
        let mut total = 0;
        while !buffer.is_empty() {
            match reader.source.read(buffer) {
                Ok(0) => break,
                Ok(count) => {
                    total += count;
                    buffer = &mut buffer[count..];
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => (),
                Err(error) => {
                    reader.error = Some(error);
                    break;
                }
            }
        }
        total
    }

    unsafe extern "C" fn seek(context: *mut c_void, offset: i64, origin: c_int) -> i64 {
        // SAFETY: The bridge calls back synchronously with the same live Reader.
        let reader = unsafe { &mut *context.cast::<Self>() };
        let position = match origin {
            0 if offset >= 0 => SeekFrom::Start(offset as u64),
            1 => SeekFrom::Current(offset),
            2 => SeekFrom::End(offset),
            _ => return -1,
        };
        match reader.source.seek(position) {
            Ok(position) => i64::try_from(position).unwrap_or(-1),
            Err(error) => {
                reader.error = Some(error);
                -1
            }
        }
    }

    unsafe extern "C" fn tag(
        context: *mut c_void,
        key: *const c_char,
        value: *const u8,
        len: usize,
    ) {
        // SAFETY: The bridge passes a C key and a borrowed buffer of len UTF-8 bytes.
        let (reader, key, value) = unsafe {
            (
                &mut *context.cast::<Self>(),
                CStr::from_ptr(key),
                slice::from_raw_parts(value, len),
            )
        };
        reader
            .snapshot
            .tags
            .entry(key.to_string_lossy().into_owned())
            .or_default()
            .push(String::from_utf8_lossy(value).into_owned());
    }

    unsafe extern "C" fn picture(
        context: *mut c_void,
        mime: *const c_char,
        description: *const c_char,
        kind: *const c_char,
        bytes: *const u8,
        len: usize,
    ) {
        // SAFETY: TagLib passes valid C strings and a borrowed image buffer for this call.
        let reader = unsafe { &mut *context.cast::<Self>() };
        let copy_string = |value| {
            unsafe { CStr::from_ptr(value) }
                .to_string_lossy()
                .into_owned()
        };
        let data = if len == 0 {
            Vec::new()
        } else {
            unsafe { slice::from_raw_parts(bytes, len) }.to_vec()
        };
        reader.snapshot.pictures.push(Picture {
            mime: copy_string(mime),
            description: copy_string(description),
            kind: copy_string(kind),
            data,
        });
    }
}

/// Text and picture data to collect alongside the stream properties.
#[derive(Debug, Clone, Copy)]
pub enum ReadFields {
    Tags,
    Pictures,
    All,
}

/// Read metadata from a file opened through the filesystem backend.
pub fn read_file(fsio: &FsIo, path: &Path, fields: ReadFields) -> Result<Snapshot> {
    let name = path
        .file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy();
    read(fsio.open(path, OpenMode::Read)?, &name, fields)
        .with_context(|| format!("reading audio metadata from {}", path.display()))
}

/// Read metadata from a seekable source using its filename as a format hint.
pub fn read<R: Read + Seek>(mut source: R, name: &str, fields: ReadFields) -> Result<Snapshot> {
    let name = CString::new(name)?;
    let length = i64::try_from(source.seek(SeekFrom::End(0))?)?;
    source.rewind()?;
    let mut reader = Reader {
        source,
        snapshot: Snapshot::default(),
        error: None,
    };
    let callbacks = Callbacks {
        read: Reader::<R>::read,
        seek: Reader::<R>::seek,
        tag: (!matches!(fields, ReadFields::Pictures)).then_some(Reader::<R>::tag),
        picture: (!matches!(fields, ReadFields::Tags)).then_some(Reader::<R>::picture),
    };

    // SAFETY: Pointers are live for this synchronous call and the bridge stores none of them.
    let mut properties = AudioProperties::default();
    let status = unsafe {
        rune_taglib_read(
            (&mut reader as *mut Reader<R>).cast(),
            name.as_ptr(),
            length,
            &callbacks,
            &mut properties,
        )
    };
    if let Some(error) = reader.error {
        return Err(error.into());
    }
    ensure!(status != 0, "unsupported or invalid audio stream");

    // The bridge returns either NULL or a pointer to a static format/codec name.
    let text = |value: *const c_char| {
        (!value.is_null()).then(|| {
            // SAFETY: Non-null pointers here point to NUL-terminated string literals.
            unsafe { CStr::from_ptr(value) }
                .to_string_lossy()
                .into_owned()
        })
    };
    let positive = |value| u32::try_from(value).ok().filter(|value| *value > 0);
    let snapshot = &mut reader.snapshot;
    snapshot.format = text(properties.format);
    snapshot.codec = text(properties.codec);
    snapshot.duration_ms = positive(properties.duration_ms).map(u64::from);
    snapshot.sample_rate = positive(properties.sample_rate);
    snapshot.channels = positive(properties.channels);
    snapshot.bitrate = positive(properties.bitrate_kbps).map(|value| u64::from(value) * 1000);
    snapshot.bits_per_sample = positive(properties.bits_per_sample);
    snapshot.frame_count = (properties.frames > 0).then_some(properties.frames);
    Ok(reader.snapshot)
}
