use std::io::{Read, Seek, SeekFrom};

use symphonia::core::io::MediaSource;

use fsio::FileStream;

pub struct FsioMediaSource {
    stream: Box<dyn FileStream>,
    size: Option<u64>,
}

impl FsioMediaSource {
    // Constructor to get the file size
    pub fn new(mut stream: Box<dyn FileStream>) -> Self {
        let size = stream.seek(SeekFrom::End(0)).ok();
        // Important: seek back to the beginning
        let _ = stream.seek(SeekFrom::Start(0));
        Self { stream, size }
    }
}

impl Read for FsioMediaSource {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.stream.read(buf)
    }
}

impl Seek for FsioMediaSource {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.stream.seek(pos)
    }
}

impl MediaSource for FsioMediaSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.size
    }
}

pub struct ReadSeekSource<T: Read + Seek + Send + Sync> {
    inner: T,
}

impl<T: Read + Seek + Send + Sync> ReadSeekSource<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> T {
        self.inner
    }

    pub fn get_ref(&self) -> &T {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

impl<T: Read + Seek + Send + Sync> MediaSource for ReadSeekSource<T> {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        None
    }
}

impl<T: Read + Seek + Send + Sync> Read for ReadSeekSource<T> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl<T: Read + Seek + Send + Sync> Seek for ReadSeekSource<T> {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.inner.seek(pos)
    }
}

static REGISTRY: once_cell::sync::Lazy<symphonia::core::codecs::CodecRegistry> =
    once_cell::sync::Lazy::new(|| {
        let mut registry = symphonia::core::codecs::CodecRegistry::new();
        register_enabled_codecs(&mut registry);
        registry
    });

/// Returns a reference to the global `CodecRegistry` populated with Symphonia's
/// default enabled codecs (excluding native AAC to prevent conflict), plus the
/// external adapters for AAC (via FDK-AAC) and Opus (via libopus).
pub fn get_codecs() -> &'static symphonia::core::codecs::CodecRegistry {
    &REGISTRY
}

/// Registers all enabled codecs: Symphonia's native decoders (FLAC, MP3, Vorbis, ALAC, PCM, ADPCM)
/// WITHOUT the native AAC decoder, and registers external adapters (FDK-AAC and libopus).
/// This ensures Symphonia's native AAC decoder does not conflict with `symphonia_adapter_fdk_aac`.
pub fn register_enabled_codecs(registry: &mut symphonia::core::codecs::CodecRegistry) {
    // 1. External adapters
    register_adapters(registry);

    // 2. Symphonia native codecs (excluding native AacDecoder)
    registry.register_all::<symphonia::default::codecs::FlacDecoder>();
    registry.register_all::<symphonia::default::codecs::MpaDecoder>();
    registry.register_all::<symphonia::default::codecs::PcmDecoder>();
    registry.register_all::<symphonia::default::codecs::VorbisDecoder>();
    registry.register_all::<symphonia::default::codecs::AlacDecoder>();
    registry.register_all::<symphonia::default::codecs::AdpcmDecoder>();
}

/// Registers external adapters (FDK AAC and libopus) on the provided registry.
pub fn register_adapters(registry: &mut symphonia::core::codecs::CodecRegistry) {
    registry.register_all::<symphonia_adapter_fdk_aac::AacDecoder>();
    registry.register_all::<symphonia_adapter_libopus::OpusDecoder>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use symphonia::core::codecs::{
        CODEC_TYPE_AAC, CODEC_TYPE_NULL, CODEC_TYPE_OPUS, DecoderOptions,
    };
    use symphonia::core::formats::FormatOptions;
    use symphonia::core::io::MediaSourceStream;
    use symphonia::core::meta::MetadataOptions;
    use symphonia::core::probe::Hint;

    #[test]
    fn test_registry_contains_adapters() {
        let registry = get_codecs();

        // Check external adapters
        assert!(
            registry.get_codec(CODEC_TYPE_OPUS).is_some(),
            "CodecRegistry should contain OPUS codec (via libopus adapter)"
        );
        let aac_desc = registry.get_codec(CODEC_TYPE_AAC);
        assert!(
            aac_desc.is_some(),
            "CodecRegistry should contain AAC codec (via fdk-aac adapter)"
        );
        assert_eq!(aac_desc.unwrap().short_name, "aac");

        // Check native codecs are still registered
        use symphonia::core::codecs::{
            CODEC_TYPE_ADPCM_MS, CODEC_TYPE_ALAC, CODEC_TYPE_FLAC, CODEC_TYPE_MP3,
            CODEC_TYPE_PCM_S16LE, CODEC_TYPE_VORBIS,
        };
        assert!(
            registry.get_codec(CODEC_TYPE_FLAC).is_some(),
            "FLAC registered"
        );
        assert!(
            registry.get_codec(CODEC_TYPE_VORBIS).is_some(),
            "Vorbis registered"
        );
        assert!(
            registry.get_codec(CODEC_TYPE_MP3).is_some(),
            "MP3 registered"
        );
        assert!(
            registry.get_codec(CODEC_TYPE_ALAC).is_some(),
            "ALAC registered"
        );
        assert!(
            registry.get_codec(CODEC_TYPE_PCM_S16LE).is_some(),
            "PCM registered"
        );
        assert!(
            registry.get_codec(CODEC_TYPE_ADPCM_MS).is_some(),
            "ADPCM registered"
        );
    }

    #[test]
    fn test_decode_opus_file() {
        let file_path = "/tmp/test_startup.opus";
        if !std::path::Path::new(file_path).exists() {
            eprintln!("Skipping test_decode_opus_file because {file_path} doesn't exist");
            return;
        }

        let file = File::open(file_path).expect("failed to open opus file");
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        hint.with_extension("opus");

        let mut probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .expect("failed to probe opus file");

        let track = probed
            .format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .expect("no valid track");

        assert_eq!(track.codec_params.codec, CODEC_TYPE_OPUS);
        let track_id = track.id;
        let codec_params = track.codec_params.clone();

        let mut decoder = get_codecs()
            .make(&codec_params, &DecoderOptions::default())
            .expect("failed to create opus decoder");

        let mut total_frames = 0;
        while let Ok(packet) = probed.format.next_packet() {
            if packet.track_id() == track_id {
                let decoded = decoder
                    .decode(&packet)
                    .expect("failed to decode opus packet");
                total_frames += decoded.frames();
            }
        }

        assert!(total_frames > 0, "Decoded zero frames from opus file!");
        println!("Successfully decoded {} frames of Opus audio", total_frames);
    }

    #[test]
    fn test_decode_aac_file() {
        let file_path = "/tmp/test_startup.m4a";
        if !std::path::Path::new(file_path).exists() {
            eprintln!("Skipping test_decode_aac_file because {file_path} doesn't exist");
            return;
        }

        let file = File::open(file_path).expect("failed to open m4a file");
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        hint.with_extension("m4a");

        let mut probed = symphonia::default::get_probe()
            .format(
                &hint,
                mss,
                &FormatOptions::default(),
                &MetadataOptions::default(),
            )
            .expect("failed to probe m4a file");

        let track = probed
            .format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .expect("no valid track");

        assert_eq!(track.codec_params.codec, CODEC_TYPE_AAC);
        let track_id = track.id;
        let codec_params = track.codec_params.clone();

        let mut decoder = get_codecs()
            .make(&codec_params, &DecoderOptions::default())
            .expect("failed to create aac decoder");

        let mut total_frames = 0;
        while let Ok(packet) = probed.format.next_packet() {
            if packet.track_id() == track_id {
                let decoded = decoder
                    .decode(&packet)
                    .expect("failed to decode aac packet");
                total_frames += decoded.frames();
            }
        }

        assert!(total_frames > 0, "Decoded zero frames from aac file!");
        println!("Successfully decoded {} frames of AAC audio", total_frames);
    }
}
