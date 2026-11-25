//! Resolve local metadata from explicit tags, streams, and folder layout.

use std::{collections::BTreeMap, path::Path, sync::LazyLock};

use anyhow::Result;
use fsio::{FsIo, FsNode};
use regex::Regex;

use crate::{
    model::{
        AlbumExtraResource, AlbumExtraResourceType, AlbumMetadata, AlbumSource, Artwork,
        ArtworkInfo, ArtworkSource, ArtworkType, AudioContentType, AudioEncodingInfo,
        AudioMetadata, Credit, Credits, DiscMetadata, FieldSource, MetadataBase, MetadataLink,
        MetadataLinkRelation, MetadataProvenance, MetadataSourceKind, RawTagMap,
        ReleaseSecondaryType, ReleaseType,
    },
    taglib::{self, Picture, ReadFields},
};

const DISC_TITLE_KEYS: &[&str] = &[
    "DISCSUBTITLE",
    "SETSUBTITLE",
    "SET SUBTITLE",
    "TSST",
    "DISCNAME",
];

use crate::credit_parser::{canonical_roles, structured_credits};

static DISC_FOLDER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:cd|disc|disque|disk)[ _.-]*(0*[1-9][0-9]*)(?:[ _.-]+(.+))?$")
        .expect("valid disc directory pattern")
});
static TRACK_NAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:([1-9][0-9]*)[-_.])?([0-9]+)[ _.-]+(.+)$")
        .expect("valid numbered track filename pattern")
});

/// Evidence contributed by one local track. Catalog identities are assigned on import.
#[derive(Debug, Clone)]
pub struct LocalMetadata {
    pub album: AlbumMetadata,
    pub disc: DiscMetadata,
    pub audio: AudioMetadata,
    pub diagnostics: ParseDiagnostics,
}

impl LocalMetadata {
    /// Expand primary artist tags for catalog membership, preserving the original credits.
    pub fn artist_names(&self) -> impl Iterator<Item = String> + '_ {
        self.audio
            .base
            .credits
            .iter()
            .flatten()
            .filter(|credit| credit.role == "Primary Artist")
            .flat_map(|credit| crate::artist::split_artists(&credit.credited_as))
    }
}

/// Raw values retained for callers that need to inspect parser decisions.
#[derive(Debug, Clone, Default)]
pub struct ParseDiagnostics {
    pub raw_tags: RawTagMap,
    pub unmapped_tags: RawTagMap,
    pub unresolved_credits: Vec<UnresolvedCredit>,
}

#[derive(Debug, Clone)]
pub struct UnresolvedCredit {
    pub role: String,
    pub names: Vec<String>,
    pub source: FieldSource,
}

struct Tags<'a> {
    raw: &'a RawTagMap,
    path: &'a str,
    separators: &'a TagSeparators<'a>,
    provenance: MetadataProvenance,
}

impl<'a> Tags<'a> {
    fn new(raw: &'a RawTagMap, path: &'a str, separators: &'a TagSeparators<'a>) -> Self {
        Self {
            raw,
            path,
            separators,
            provenance: BTreeMap::new(),
        }
    }

    fn read<T>(
        &mut self,
        field: &str,
        keys: &[&str],
        parse: impl Fn(&str) -> Option<T>,
    ) -> Option<T> {
        for key in keys {
            for (raw_key, values) in self
                .raw
                .iter()
                .filter(|(raw_key, _)| matching_key(raw_key).eq_ignore_ascii_case(key))
            {
                if let Some(value) = values
                    .iter()
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                    .find_map(&parse)
                {
                    self.record(field, raw_key);
                    return Some(value);
                }
            }
        }
        None
    }

    fn record(&mut self, field: &str, key: &str) {
        self.provenance
            .entry(field.to_owned())
            .or_default()
            .push(FieldSource {
                key: Some(key.to_owned()),
                ..source(MetadataSourceKind::AudioTag, self.path, None)
            });
    }

    fn text(&mut self, field: &str, keys: &[&str]) -> Option<String> {
        self.read(field, keys, |value| Some(value.to_owned()))
    }

    fn field_separators(&self, field: &str) -> &'a [&'a str] {
        self.separators
            .iter()
            .find_map(|&(name, separators)| (name == field).then_some(separators))
            .unwrap_or_default()
    }

    fn list(&mut self, field: &str, keys: &[&str]) -> Option<Vec<String>> {
        let mut result = Vec::new();
        let separators = self.field_separators(field);
        for (key, values) in self.raw {
            if keys
                .iter()
                .any(|alias| matching_key(key).eq_ignore_ascii_case(alias))
            {
                let previous = result.len();
                for name in values
                    .iter()
                    .flat_map(|value| crate::delimiters::split(value, separators))
                    .map(|value| value.trim())
                    .filter(|value| !value.is_empty())
                {
                    if !result.iter().any(|existing| existing == name) {
                        result.push(name.to_owned());
                    }
                }
                if result.len() > previous {
                    self.record(field, key);
                }
            }
        }
        (!result.is_empty()).then_some(result)
    }

    fn finish(self, base: &mut MetadataBase) {
        base.provenance
            .get_or_insert_default()
            .extend(self.provenance);
    }
}

fn matching_key(key: &str) -> &str {
    let key = key.trim();
    ["TXXX:", "----:com.apple.iTunes:", "freeform:"]
        .iter()
        .find_map(|prefix| key.strip_prefix(prefix))
        .unwrap_or(key)
}

pub(crate) fn source(kind: MetadataSourceKind, path: &str, pattern: Option<&str>) -> FieldSource {
    FieldSource {
        kind,
        path: Some(path.to_owned()),
        pattern: pattern.map(str::to_owned),
        key: None,
        entity_id: None,
        confidence: None,
    }
}

pub(crate) fn set_source(base: &mut MetadataBase, field: &str, source: FieldSource) {
    base.provenance
        .get_or_insert_default()
        .insert(field.to_owned(), vec![source]);
}

fn positive(value: &str) -> Option<u32> {
    value.trim().parse().ok().filter(|value| *value > 0)
}

fn partial_date(value: &str) -> Option<String> {
    match value.len() {
        4 if value.bytes().all(|byte| byte.is_ascii_digit()) => Some(value.to_owned()),
        7 => format!("{value}-01")
            .parse::<jiff::civil::Date>()
            .ok()
            .map(|_| value.to_owned()),
        10 => value
            .parse::<jiff::civil::Date>()
            .ok()
            .map(|date| date.to_string()),
        _ => value
            .parse::<jiff::Timestamp>()
            .ok()
            .and_then(|_| value.get(..10))
            .and_then(partial_date),
    }
}

fn number(value: &str) -> Option<f64> {
    value
        .strip_suffix(" dB")
        .or_else(|| value.strip_suffix(" LUFS"))
        .unwrap_or(value)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
}

fn tagged_bool(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        _ => None,
    }
}

fn embedded_artwork(pictures: Vec<Picture>, path: &str) -> Vec<Artwork> {
    pictures
        .into_iter()
        .enumerate()
        .filter_map(|(index, picture)| {
            let kind = match picture.kind.to_ascii_lowercase().as_str() {
                "front cover" | "cover (front)" => ArtworkType::Front,
                "back cover" | "cover (back)" => ArtworkType::Back,
                "media" => ArtworkType::Disc,
                "leaflet page" => ArtworkType::Booklet,
                "artist/performer" | "lead artist/lead performer/soloist" => ArtworkType::Artist,
                _ => ArtworkType::Other,
            };
            let dimensions = image::ImageReader::new(std::io::Cursor::new(&picture.data))
                .with_guessed_format()
                .ok()
                .and_then(|image| image.into_dimensions().ok());
            Some(Artwork {
                info: ArtworkInfo {
                    kind,
                    description: nonempty(picture.description),
                    mime_type: nonempty(picture.mime),
                    width: dimensions.map(|size| size.0),
                    height: dimensions.map(|size| size.1),
                },
                source: ArtworkSource::Embedded {
                    audio_path: path.to_owned(),
                    picture_index: index.try_into().ok()?,
                },
            })
        })
        .collect()
}

fn nonempty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

impl LocalMetadata {
    /// Discover artwork and immediate album attachment directories.
    pub async fn discover_resources(&mut self, fsio: &FsIo) {
        let AlbumSource::Local { directory_path } = &self.album.source else {
            return;
        };
        if let Ok(mut entries) = fsio.read_dir(Path::new(directory_path)).await {
            entries.sort_unstable_by(|a, b| a.filename.cmp(&b.filename));
            let scans: Vec<_> = entries
                .iter()
                .filter(|entry| entry.is_dir && entry.filename.trim().eq_ignore_ascii_case("scans"))
                .map(|entry| AlbumExtraResource {
                    kind: AlbumExtraResourceType::Scans,
                    path: entry.path.to_string_lossy().into_owned(),
                })
                .collect();
            if !scans.is_empty() {
                let evidence = scans
                    .iter()
                    .map(|scan| {
                        source(
                            MetadataSourceKind::Directory,
                            &scan.path,
                            Some("scans-folder"),
                        )
                    })
                    .collect();
                self.album
                    .base
                    .provenance
                    .get_or_insert_default()
                    .insert("extra_resources".into(), evidence);
                self.album.extra_resources = Some(scans);
            }
            discover_artwork(fsio, entries, &mut self.album.base).await;
        }

        if let Some(path) = &self.disc.directory_path
            && let Ok(mut entries) = fsio.read_dir(Path::new(path)).await
        {
            entries.sort_unstable_by(|a, b| a.filename.cmp(&b.filename));
            discover_artwork(fsio, entries, &mut self.disc.base).await;
        }
    }
}

async fn discover_artwork(fsio: &FsIo, entries: Vec<FsNode>, base: &mut MetadataBase) {
    // Remote FsIo streams must be opened off the async runtime.
    let fsio = fsio.clone();
    let artwork = tokio::task::spawn_blocking(move || sidecar_artwork(&fsio, &entries))
        .await
        .unwrap_or_default();
    if !artwork.is_empty() {
        let evidence = artwork
            .iter()
            .filter_map(|artwork| match &artwork.source {
                ArtworkSource::File { path } => Some(source(
                    MetadataSourceKind::Sidecar,
                    path,
                    Some("artwork-filename"),
                )),
                _ => None,
            })
            .collect();
        base.artwork = Some(artwork);
        base.provenance
            .get_or_insert_default()
            .insert("artwork".into(), evidence);
    }
}

fn sidecar_artwork(fsio: &FsIo, entries: &[FsNode]) -> Vec<Artwork> {
    entries
        .iter()
        .filter(|entry| entry.is_file)
        .filter_map(|entry| {
            let stem = entry.path.file_stem()?.to_str()?.to_ascii_lowercase();
            let kind = match stem.as_str() {
                "cover" | "folder" | "front" | "albumart" | "coverart" => ArtworkType::Front,
                "back" => ArtworkType::Back,
                "disc" => ArtworkType::Disc,
                "booklet" => ArtworkType::Booklet,
                "artist" => ArtworkType::Artist,
                _ => return None,
            };

            // Validate the real image type and read dimensions without decoding pixels.
            let bytes = fsio.open(&entry.path, fsio::OpenMode::Read).ok()?;
            let reader = image::ImageReader::new(std::io::BufReader::new(bytes))
                .with_guessed_format()
                .ok()?;
            let mime_type = reader.format()?.to_mime_type().to_owned();
            let (width, height) = reader.into_dimensions().ok()?;
            Some(Artwork {
                info: ArtworkInfo {
                    kind,
                    description: Some(entry.filename.clone()),
                    mime_type: Some(mime_type),
                    width: Some(width),
                    height: Some(height),
                },
                source: ArtworkSource::File {
                    path: entry.path.to_string_lossy().into_owned(),
                },
            })
        })
        .collect()
}

/// Read and resolve the metadata of a local audio file.
pub fn read_local(fsio: &FsIo, node: &FsNode, root: &Path) -> Result<LocalMetadata> {
    read_local_with_separators(fsio, node, root, &[])
}

/// Optional literal separators keyed by normalized list field, such as
/// `credits.Primary Artist` or `genres`.
pub type TagSeparators<'a> = [(&'a str, &'a [&'a str])];

/// Read and resolve local metadata with caller-supplied list separators.
pub fn read_local_with_separators(
    fsio: &FsIo,
    node: &FsNode,
    root: &Path,
    separators: &TagSeparators<'_>,
) -> Result<LocalMetadata> {
    read_location(fsio, node, root, separators, true)
}

/// Read a cached download whose filesystem name and directory are transport addresses.
pub fn read_managed_audio(
    fsio: &FsIo,
    node: &FsNode,
    cache_directory: &Path,
) -> Result<LocalMetadata> {
    read_location(fsio, node, cache_directory, &[], false)
}

fn read_location(
    fsio: &FsIo,
    node: &FsNode,
    root: &Path,
    separators: &TagSeparators<'_>,
    use_path_evidence: bool,
) -> Result<LocalMetadata> {
    let path = node.path.to_string_lossy();
    let snapshot = taglib::read_file(fsio, &node.path, ReadFields::All)?;
    let mut resolved = resolve(&snapshot.tags, node, root, separators, use_path_evidence);
    resolved.diagnostics.raw_tags = snapshot.tags;
    let audio = &mut resolved.audio;

    // The stream overrides technical values inferred from text.
    audio.format = snapshot.format;
    audio.codec = snapshot.codec;
    audio.sample_rate = snapshot.sample_rate;
    audio.bits_per_sample = snapshot.bits_per_sample;
    audio.channels = snapshot.channels;
    audio.frame_count = snapshot.frame_count;
    audio.bitrate = snapshot.bitrate;
    audio.base.duration_ms = snapshot.duration_ms.or(audio.base.duration_ms);

    // Track each resolved audio stream property independently.
    for (field, present) in [
        ("format", audio.format.is_some()),
        ("codec", audio.codec.is_some()),
        ("sample_rate", audio.sample_rate.is_some()),
        ("bits_per_sample", audio.bits_per_sample.is_some()),
        ("channels", audio.channels.is_some()),
        ("frame_count", audio.frame_count.is_some()),
        ("bitrate", audio.bitrate.is_some()),
        ("duration_ms", snapshot.duration_ms.is_some()),
    ] {
        if present {
            set_source(
                &mut audio.base,
                field,
                source(MetadataSourceKind::AudioStream, &path, None),
            );
        }
    }

    // Embedded image references use the native picture order.
    let artwork = embedded_artwork(snapshot.pictures, &path);
    if !artwork.is_empty() {
        audio.base.artwork = Some(artwork);
        set_source(
            &mut audio.base,
            "artwork",
            source(MetadataSourceKind::EmbeddedArtwork, &path, None),
        );
    }
    audio.file_modified_at = fsio
        .modified_time(&node.path)
        .ok()
        .and_then(|seconds| jiff::Timestamp::from_second(seconds.try_into().ok()?).ok())
        .map(|timestamp| timestamp.to_string());

    Ok(resolved)
}

pub(crate) fn folder_layout<'a>(
    path: &'a Path,
    root: &'a Path,
) -> (&'a Path, Option<u32>, Option<String>) {
    let parent = path.parent().unwrap_or(root);
    let matching = parent
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| DISC_FOLDER.captures(name));
    if parent != root
        && let Some(parts) = matching
        && let Some(number) = positive(&parts[1])
        && let Some(album) = parent.parent().filter(|album| album.starts_with(root))
    {
        return (
            album,
            Some(number),
            parts.get(2).map(|part| part.as_str().to_owned()),
        );
    }
    (parent, None, None)
}

impl AlbumMetadata {
    /// Fill an untagged local release's version from its own directory name.
    pub fn infer_directory_version(&mut self) {
        let AlbumSource::Local { directory_path } = &self.source else {
            return;
        };
        let inferred = self
            .base
            .provenance
            .as_ref()
            .and_then(|fields| fields.get("version"))
            .is_some_and(|sources| {
                sources.iter().any(|source| {
                    source.kind == MetadataSourceKind::Directory
                        && source.pattern.as_deref() == Some("album-folder-version")
                })
            });
        if self.base.version.is_some() && !inferred {
            return;
        }
        let Some(name) = Path::new(directory_path)
            .file_name()
            .and_then(|name| name.to_str())
        else {
            return;
        };

        // Keep nonblank bracketed values in their original order.
        let versions: Vec<_> = name
            .split('[')
            .skip(1)
            .filter_map(|part| part.split_once(']').map(|(version, _)| version.trim()))
            .filter(|version| !version.is_empty())
            .collect();
        if !versions.is_empty() {
            self.base.version = Some(versions.join(", "));
            set_source(
                &mut self.base,
                "version",
                source(
                    MetadataSourceKind::Directory,
                    directory_path,
                    Some("album-folder-version"),
                ),
            );
        }
    }
}

fn resolve(
    raw: &RawTagMap,
    node: &FsNode,
    root: &Path,
    separators: &TagSeparators<'_>,
    use_path_evidence: bool,
) -> LocalMetadata {
    let path = node.path.to_string_lossy();
    let (directory, folder_disc, folder_title) = if use_path_evidence {
        folder_layout(&node.path, root)
    } else {
        (root, None, None)
    };
    let dir = directory.to_string_lossy();
    let stem = node
        .path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(&node.filename);
    let filename = use_path_evidence
        .then(|| TRACK_NAME.captures(stem))
        .flatten();

    // Resolve each scope separately so album-specific credit and title tags stay on the release.
    let mut album_tags = Tags::new(raw, &path, separators);
    let mut disc_tags = Tags::new(raw, &path, separators);
    let mut audio_tags = Tags::new(raw, &path, separators);
    let mut album = album(&mut album_tags, &dir);
    let mut audio = audio(&mut audio_tags, &path, node.size);
    let mut disc = DiscMetadata {
        base: shared(&mut disc_tags),
        ..Default::default()
    };

    if album.title.is_empty() && use_path_evidence {
        album.title = directory
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&dir)
            .to_owned();
        set_source(
            &mut album.base,
            "title",
            source(MetadataSourceKind::Directory, &dir, Some("album-folder")),
        );
    }
    if audio.title.is_empty() && use_path_evidence {
        // Plain filenames retain their extension, while numbered names supply a track title.
        audio.title = filename
            .as_ref()
            .map(|parts| &parts[3])
            .unwrap_or(&node.filename)
            .to_owned();
        set_source(
            &mut audio.base,
            "title",
            source(MetadataSourceKind::Filename, &path, Some("track-title")),
        );
    }
    if album.title.is_empty() {
        album.title = "Unknown Album".into();
    }
    if audio.title.is_empty() {
        audio.title = "Unknown Title".into();
    }

    // Prefer numbers in tags, then a conventional disc directory, then filename numbering.
    if audio.track_number.is_none()
        && let Some(parts) = &filename
    {
        audio.track_number = positive(&parts[2]);
        if audio.track_number.is_some() {
            set_source(
                &mut audio.base,
                "track_number",
                source(MetadataSourceKind::Filename, &path, Some("numbered-track")),
            );
        }
    }
    if audio.disc_number.is_none() {
        let filename_disc = filename
            .as_ref()
            .and_then(|parts| positive(parts.get(1)?.as_str()));
        audio.disc_number = folder_disc.or(filename_disc);
        if audio.disc_number.is_some() {
            let evidence = if folder_disc.is_some() {
                source(
                    MetadataSourceKind::Directory,
                    &node.path.parent().unwrap_or(root).to_string_lossy(),
                    Some("disc-folder"),
                )
            } else {
                source(MetadataSourceKind::Filename, &path, Some("disc-track"))
            };
            set_source(&mut audio.base, "disc_number", evidence);
        }
    }

    // Preserve title alias priority for the later cross-track resolver.
    disc.title = disc_tags.text("title", DISC_TITLE_KEYS);
    if disc.title.is_none() && folder_title.is_some() {
        disc.title = folder_title;
        set_source(
            &mut disc.base,
            "title",
            source(
                MetadataSourceKind::Directory,
                &node.path.parent().unwrap_or(root).to_string_lossy(),
                Some("disc-folder"),
            ),
        );
    }
    disc.media_format = disc_tags.text(
        "media_format",
        &["MEDIA", "MEDIAFORMAT", "MEDIA_FORMAT", "TMED"],
    );
    disc.base.identifiers = identifiers(
        &mut disc_tags,
        &[(
            "musicbrainz:disc",
            &["MUSICBRAINZ_DISCID", "MUSICBRAINZ DISC ID"],
        )],
    );
    disc.declared_track_count = audio.track_total;
    album.disc_count = audio.disc_total;
    disc.directory_path = folder_disc.map(|_| {
        node.path
            .parent()
            .unwrap_or(root)
            .to_string_lossy()
            .into_owned()
    });

    // Provide the numbering and evidence. The catalog resolves the persistent parent IDs.
    disc.disc_number = audio.disc_number.unwrap_or(1);
    album_tags.finish(&mut album.base);
    if use_path_evidence {
        album.infer_directory_version();
    }
    disc_tags.finish(&mut disc.base);
    audio_tags.finish(&mut audio.base);
    let unmapped_tags = unmapped_tags(raw, [&album.base, &disc.base, &audio.base]);
    LocalMetadata {
        album,
        disc,
        audio,
        diagnostics: ParseDiagnostics {
            unmapped_tags,
            unresolved_credits: unresolved_credits(raw, &path),
            ..Default::default()
        },
    }
}

fn shared(tags: &mut Tags<'_>) -> MetadataBase {
    MetadataBase {
        genres: tags.list("genres", &["GENRE", "GENRES", "STYLE", "STYLES"]),
        release_date: tags.read(
            "release_date",
            &["RELEASEDATE", "TDRL", "DATE", "YEAR", "TYER"],
            partial_date,
        ),
        original_release_date: tags.read(
            "original_release_date",
            &["ORIGINALDATE", "ORIGINALRELEASEDATE", "TDOR", "TORY"],
            partial_date,
        ),
        version: tags.text("version", &["VERSION"]),
        grouping: tags.text("grouping", &["GROUPING", "CONTENTGROUP", "GRP1"]),
        moods: tags.list("moods", &["MOOD", "TMOO"]),

        // Remaining optional tags in this scope.
        description: tags.text("description", &["DESCRIPTION"]),
        comment: tags.text("comment", &["COMMENT", "COMM"]),
        copyright: tags.text("copyright", &["COPYRIGHT", "TCOP"]),
        license: tags.text("license", &["LICENSE"]),
        languages: tags.list("languages", &["LANGUAGE", "LANGUAGES", "TLAN"]),
        script: tags.text("script", &["SCRIPT"]),

        // Remaining optional tags in this scope.
        work: tags.text("work", &["WORK", "COMPOSITION"]),
        part: tags.text("part", &["PART", "MOVEMENTNAME", "MOVEMENT"]),
        section: tags.text("section", &["SECTION"]),
        opus: tags.text("opus", &["OPUS"]),
        work_id: tags.text("work_id", &["WORKID", "MUSICBRAINZ_WORKID"]),
        user_tags: tags.list("user_tags", &["USERTAGS"]),

        // Remaining optional tags in this scope.
        recording_date: tags.read(
            "recording_date",
            &["RECORDINGDATE", "RECORDING DATE", "PERFORMANCEDATE", "TDRC"],
            partial_date,
        ),
        recording_start_date: tags.read(
            "recording_start_date",
            &["RECORDINGSTARTDATE", "PERFORMANCESTARTDATE"],
            partial_date,
        ),
        recording_end_date: tags.read(
            "recording_end_date",
            &["RECORDINGENDDATE", "PERFORMANCEENDDATE"],
            partial_date,
        ),
        recording_location: tags.text(
            "recording_location",
            &["RECORDINGLOCATION", "PERFORMANCELOCATION", "LOCATION"],
        ),
        is_live: tags.read("is_live", &["LIVE"], tagged_bool),
        is_bootleg: tags.read("is_bootleg", &["BOOTLEG"], tagged_bool),
        ..Default::default()
    }
}

fn album(tags: &mut Tags<'_>, directory: &str) -> AlbumMetadata {
    let mut base = shared(tags);
    base.version = tags.text("version", &["ALBUMVERSION", "RELEASEVERSION"]);
    if base.version.is_none() {
        tags.provenance.remove("version");
    }
    base.sort_title = tags.text("sort_title", &["ALBUMSORT", "ALBUMSORTORDER", "TSOA"]);
    base.credits = credits(
        tags,
        &["ALBUMARTIST", "ALBUM_ARTIST", "ALBUM ARTIST", "TPE2"],
    );
    base.links = links(tags);
    base.identifiers = identifiers(
        tags,
        &[
            (
                "musicbrainz:release",
                &["MUSICBRAINZ_ALBUMID", "MUSICBRAINZ ALBUM ID"],
            ),
            (
                "musicbrainz:release-group",
                &["MUSICBRAINZ_RELEASEGROUPID", "MUSICBRAINZ RELEASE GROUP ID"],
            ),
            (
                "musicbrainz:album-artist",
                &["MUSICBRAINZ_ALBUMARTISTID", "MUSICBRAINZ ALBUM ARTIST ID"],
            ),
            ("asin:product", &["ASIN"]),
        ],
    );
    let (release_type, release_secondary_types) = release_types(tags);
    AlbumMetadata {
        base,
        needs_review: false,
        title: tags
            .text("title", &["ALBUM", "TALB", "©alb"])
            .unwrap_or_default(),
        source: AlbumSource::Local {
            directory_path: directory.to_owned(),
        },
        extra_resources: None,
        disc_count: None,
        total_tracks: None,

        // Only explicit album-wide totals belong on this entity.
        barcode: tags.text("barcode", &["BARCODE", "UPC", "EAN/UPN", "PRODUCTCODE"]),
        catalog_number: tags.text(
            "catalog_number",
            &[
                "CATALOGNUMBER",
                "CATALOG",
                "CATALOGUE",
                "CATALOGNO",
                "LABELNO",
            ],
        ),
        labels: tags.list("labels", &["LABEL", "ORGANIZATION", "PUBLISHER", "TPUB"]),
        release_country: tags.text(
            "release_country",
            &[
                "RELEASECOUNTRY",
                "COUNTRY",
                "MUSICBRAINZ ALBUM RELEASE COUNTRY",
            ],
        ),
        release_type,
        release_secondary_types,
        release_status: tags.text(
            "release_status",
            &[
                "RELEASESTATUS",
                "MUSICBRAINZ_ALBUMSTATUS",
                "MUSICBRAINZ ALBUM STATUS",
            ],
        ),

        // Remaining optional tags in this scope.
        declared_track_count: tags.read(
            "declared_track_count",
            &["ALBUMTRACKTOTAL", "ALBUMTOTALTRACKS"],
            positive,
        ),
        replaygain_album_gain_db: tags.read(
            "replaygain_album_gain_db",
            &["REPLAYGAIN_ALBUM_GAIN"],
            number,
        ),

        // Remaining optional tags in this scope.
        replaygain_album_peak: tags.read(
            "replaygain_album_peak",
            &["REPLAYGAIN_ALBUM_PEAK"],
            number,
        ),
        replaygain_reference_lufs: tags.read(
            "replaygain_reference_lufs",
            &["REPLAYGAIN_REFERENCE_LOUDNESS"],
            number,
        ),
    }
}

fn release_types(
    tags: &mut Tags<'_>,
) -> (Option<ReleaseType>, BTreeMap<ReleaseSecondaryType, bool>) {
    use std::collections::BTreeSet;

    let mut primary = BTreeSet::new();
    let mut secondary = BTreeMap::new();
    let types = tags.list(
        "release_type",
        &[
            "RELEASETYPE",
            "MUSICBRAINZ_ALBUMTYPE",
            "MUSICBRAINZ ALBUM TYPE",
        ],
    );
    // Picard may write multiple values or a combined type such as album; compilation.
    for value in types.iter().flatten() {
        if let Some(kind) = ReleaseSecondaryType::from_label(value) {
            secondary.insert(kind, true);
            continue;
        }
        for token in value.split([';', ',']) {
            if let Some(kind) = ReleaseType::from_label(token) {
                primary.insert(kind);
            } else if let Some(kind) = ReleaseSecondaryType::from_label(token) {
                secondary.insert(kind, true);
            }
        }
    }

    // Boolean legacy tags contribute only when they can represent a consistent primary type.
    if primary.is_empty() {
        for (key, kind) in [
            ("ISSINGLE", ReleaseType::Single),
            ("ISEXTENDEDPLAY", ReleaseType::Ep),
        ] {
            if tags.read("release_type", &[key], tagged_bool) == Some(true) {
                primary.insert(kind);
            }
        }
    }
    let type_sources = tags.provenance.get("release_type").cloned();
    for &kind in secondary.keys() {
        if let Some(sources) = &type_sources {
            tags.provenance.insert(
                format!("release_secondary_types.{}", kind.as_str()),
                sources.clone(),
            );
        }
    }

    // Dedicated flags preserve explicit false and take precedence within a file.
    for (kind, keys) in [
        (
            ReleaseSecondaryType::Compilation,
            &["COMPILATION", "ISCOMPILATION", "TCMP"][..],
        ),
        (ReleaseSecondaryType::Soundtrack, &["ISSOUNDTRACK"][..]),
        (ReleaseSecondaryType::Live, &["ISLIVE", "LIVE"][..]),
    ] {
        let field = format!("release_secondary_types.{}", kind.as_str());
        if let Some(enabled) = tags.read(&field, keys, tagged_bool) {
            secondary.insert(kind, enabled);
        }
    }
    (
        (primary.len() == 1).then(|| primary.pop_first()).flatten(),
        secondary,
    )
}

fn audio(tags: &mut Tags<'_>, path: &str, size: u64) -> AudioMetadata {
    let mut base = shared(tags);
    base.sort_title = tags.text("sort_title", &["TITLESORT", "TITLESORTORDER", "TSOT"]);
    base.credits = credits(tags, &["ARTIST", "TPE1", "©ART"]);
    base.duration_ms = tags.read("duration_ms", &["LENGTH", "TLEN"], |value| {
        positive(value).map(u64::from)
    });

    // Recording IDs stay on audio entities.
    base.identifiers = identifiers(
        tags,
        &[
            (
                "musicbrainz:recording",
                &[
                    "MUSICBRAINZ_TRACKID",
                    "MUSICBRAINZ_TRACK_ID",
                    "MUSICBRAINZ_RECORDINGID",
                ],
            ),
            (
                "musicbrainz:release-track",
                &["MUSICBRAINZ_RELEASETRACKID", "MUSICBRAINZ RELEASE TRACK ID"],
            ),
            ("acoustid:id", &["ACOUSTID_ID", "ACOUSTID ID"]),
            (
                "musicbrainz:artist",
                &["MUSICBRAINZ_ARTISTID", "MUSICBRAINZ ARTIST ID"],
            ),
        ],
    );
    base.links = links(tags);

    let mut audio = AudioMetadata {
        base,
        title: tags
            .text("title", &["TITLE", "TIT2", "©nam"])
            .unwrap_or_default(),
        file_path: Some(path.to_owned()),
        file_size_bytes: Some(size),

        // Parsed textual and numeric attributes specific to the recording.
        subtitle: tags.text("subtitle", &["SUBTITLE", "TRACKSUBTITLE", "TIT3"]),
        bpm: tags.read("bpm", &["BPM", "TBPM"], number),
        movement_number: tags.read(
            "movement_number",
            &["MOVEMENTNUMBER", "MOVEMENT", "MVIN"],
            positive,
        ),
        movement_total: tags.read(
            "movement_total",
            &["MOVEMENTCOUNT", "MOVEMENTTOTAL"],
            positive,
        ),
        replaygain_track_gain_db: tags.read(
            "replaygain_track_gain_db",
            &["REPLAYGAIN_TRACK_GAIN"],
            number,
        ),
        replaygain_track_peak: tags.read(
            "replaygain_track_peak",
            &["REPLAYGAIN_TRACK_PEAK"],
            number,
        ),

        // Remaining optional tags in this scope.
        replaygain_reference_lufs: tags.read(
            "replaygain_reference_lufs",
            &["REPLAYGAIN_REFERENCE_LOUDNESS"],
            number,
        ),
        lyrics: tags.text("lyrics", &["LYRICS", "UNSYNCED LYRICS", "USLT"]),
        synced_lyrics: tags.text("synced_lyrics", &["SYLT", "SYNCED LYRICS", "SYNCEDLYRICS"]),
        original_album_title: tags.text(
            "original_album_title",
            &["ORIGINALALBUM", "ORIGINAL ALBUM", "TOAL"],
        ),
        isrc: tags.text("isrc", &["ISRC", "TSRC"]),
        content_type: tags.read("content_type", &["CONTENTTYPE"], content_type),
        ..Default::default()
    };

    let tracks = &["TRACKNUMBER", "TRACK", "TRCK", "trkn"];
    let discs = &["DISCNUMBER", "DISC", "TPOS", "disk"];
    let index = |value: &str| positive(value.split('/').next()?);
    let total = |value: &str| positive(value.split_once('/')?.1);
    audio.track_number = tags.read("track_number", tracks, index);
    audio.disc_number = tags.read("disc_number", discs, index);
    audio.track_total = tags
        .read(
            "track_total",
            &["TRACKTOTAL", "TOTALTRACKS", "TRACKCOUNT"],
            positive,
        )
        .or_else(|| tags.read("track_total", tracks, total));
    audio.disc_total = tags
        .read(
            "disc_total",
            &["DISCTOTAL", "TOTALDISCS", "DISCCOUNT"],
            positive,
        )
        .or_else(|| tags.read("disc_total", discs, total));

    // Fingerprints and encoder details have dedicated fields.
    audio.fingerprints = tags
        .text(
            "fingerprints.chromaprint",
            &["ACOUSTID_FINGERPRINT", "ACOUSTID FINGERPRINT"],
        )
        .map(|value| BTreeMap::from([("chromaprint".to_owned(), value)]));
    let timestamp = |value: &str| {
        value
            .parse::<jiff::Timestamp>()
            .ok()
            .map(|date| date.to_string())
    };
    let encoding = AudioEncodingInfo {
        encoder: tags.text("encoding.encoder", &["ENCODER", "ENCODEDBY", "ENCODING"]),
        encoder_settings: tags.text("encoding.encoder_settings", &["ENCODERSETTINGS"]),
        encoding_date: tags.read(
            "encoding.encoding_date",
            &["ENCODINGTIME", "ENCODINGDATE", "TDEN"],
            timestamp,
        ),
        tagging_date: tags.read("encoding.tagging_date", &["TAGGINGDATE", "TDTG"], timestamp),
    };
    audio.encoding = (encoding != AudioEncodingInfo::default()).then_some(encoding);
    if audio.content_type.is_none() {
        audio.content_type = tags
            .read("content_type", &["PODCAST", "ISPODCAST"], |value| {
                (tagged_bool(value) == Some(true)).then_some(AudioContentType::Podcast)
            })
            .or_else(|| {
                tags.read("content_type", &["ISAUDIOBOOK"], |value| {
                    (tagged_bool(value) == Some(true)).then_some(AudioContentType::Audiobook)
                })
            });
    }
    audio
}

fn content_type(value: &str) -> Option<AudioContentType> {
    match value.to_ascii_lowercase().as_str() {
        "music" => Some(AudioContentType::Music),
        "podcast" => Some(AudioContentType::Podcast),
        "audiobook" => Some(AudioContentType::Audiobook),
        "other" => Some(AudioContentType::Other),
        _ => None,
    }
}

fn credits(tags: &mut Tags<'_>, primary: &[&str]) -> Option<Credits> {
    let mut credits = Vec::new();
    let roles: &[(&str, &[&str])] = &[
        ("Primary Artist", primary),
        ("Featured Artist", &["FEATURING"]),
        ("Composer", &["COMPOSER", "TCOM"]),
        ("Lyricist", &["LYRICIST", "TEXT"]),
        ("Librettist", &["LIBRETTO", "LIBRETTIST"]),
        ("Soloist", &["SOLOIST", "SOLOISTS"]),
        ("Vocals", &["VOCALS", "VOCALIST"]),
        (
            "Background Vocals",
            &["BACKING VOCALS", "BACKGROUND VOCALS"],
        ),
        ("Remixing", &["REMIXER", "REMIXED BY", "MIXARTIST", "TPE4"]),
        ("Mix-DJ", &["DJMIXER"]),
        ("Mixing", &["MIXER"]),
        ("Arranger", &["ARRANGER"]),
        ("Ensemble", &["ENSEMBLE"]),
        ("Orchestra", &["ORCHESTRA"]),
        ("Engineer", &["ENGINEER"]),
        ("Author", &["AUTHOR"]),
        ("Conductor", &["CONDUCTOR", "TPE3"]),
        ("Producer", &["PRODUCER"]),
    ];
    for (role, keys) in roles {
        for (key, names) in tags.raw {
            if keys
                .iter()
                .any(|alias| matching_key(key).eq_ignore_ascii_case(alias))
            {
                for name in names {
                    add_credit(tags, &mut credits, key, name, role, None);
                }
            }
        }
    }

    // Preserve instruments from dedicated tags and recognized personnel descriptions.
    for (key, values) in tags.raw {
        for value in values {
            for credit in structured_credits(matching_key(key), value) {
                let roles = canonical_roles(&credit.role);
                if let Some(instrument) =
                    explicit_instrument(key, &credit.role).or_else(|| personnel_instrument(roles))
                {
                    add_credit(
                        tags,
                        &mut credits,
                        key,
                        credit.name,
                        "Performer",
                        Some(instrument),
                    );
                    continue;
                }

                // Keep custom roles when the vocabulary does not recognize the description.
                let roles = if roles.is_empty() {
                    &[credit.role.as_ref()]
                } else {
                    roles
                };
                for role in roles {
                    add_credit(tags, &mut credits, key, credit.name, role, None);
                }
            }
        }
    }
    (!credits.is_empty()).then_some(credits)
}

fn explicit_instrument<'a>(key: &str, role: &'a str) -> Option<&'a str> {
    let tagged = if let Some((base, qualifier)) = role.split_once(" [")
        && base.eq_ignore_ascii_case("performer")
    {
        qualifier.strip_suffix(']')?.trim()
    } else {
        let key = matching_key(key);
        let prefix = key.split_once(':').map(|(prefix, _)| prefix);
        ["PERFORMER", "TMCL", "MUSICIANCREDITS"]
            .iter()
            .any(|instrument_key| {
                prefix.is_some_and(|prefix| prefix.eq_ignore_ascii_case(instrument_key))
            })
            .then_some(role)?
    };

    // A unique vocabulary match gives the canonical spelling for an explicitly tagged instrument.
    match canonical_roles(tagged) {
        [instrument] => Some(instrument),
        _ => (!tagged.is_empty()).then_some(tagged),
    }
}

fn personnel_instrument(roles: &[&'static str]) -> Option<&'static str> {
    // Generic personnel fields also contain production and vocal roles. Require a unique
    // vocabulary match and a recognizable instrument name for those fields.
    let [instrument] = roles else {
        return None;
    };
    let family = instrument.split_whitespace().next_back()?;
    (matches!(
        family,
        "Bassoon"
            | "Cello"
            | "Clarinet"
            | "Drums"
            | "Flute"
            | "Guitar"
            | "Harp"
            | "Keyboards"
            | "Oboe"
            | "Organ"
            | "Percussion"
            | "Piano"
            | "Saxophone"
            | "Synthesizer"
            | "Trombone"
            | "Trumpet"
            | "Tuba"
            | "Viola"
            | "Violin"
    ) || *instrument == "Chapman Stick")
        .then_some(instrument)
}

fn add_credit(
    tags: &mut Tags<'_>,
    credits: &mut Credits,
    key: &str,
    value: &str,
    role: &str,
    instrument: Option<&str>,
) {
    let field = format!("credits.{role}");
    let instrument_field = instrument.map(|name| format!("credits.{name}"));
    let separators = tags
        .separators
        .iter()
        .find_map(|&(name, separators)| {
            (Some(name) == instrument_field.as_deref()).then_some(separators)
        })
        .unwrap_or_else(|| tags.field_separators(&field));
    let mut added = false;
    for name in crate::delimiters::split(value, separators)
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        if credits.iter().any(|credit| {
            credit.credited_as == name
                && credit.role == role
                && credit.instrument.as_deref() == instrument
        }) {
            continue;
        }
        credits.push(Credit {
            credited_as: name.to_owned(),
            role: role.to_owned(),
            artist_id: None,
            instrument: instrument.map(str::to_owned),
            source: FieldSource {
                key: Some(key.to_owned()),
                ..source(MetadataSourceKind::AudioTag, tags.path, None)
            },
            source_provider: None,
        });
        added = true;
    }
    if added {
        tags.record(&field, key);
    }
}

fn unmapped_tags(raw: &RawTagMap, bases: [&MetadataBase; 3]) -> RawTagMap {
    let mapped: std::collections::HashSet<_> = bases
        .into_iter()
        .filter_map(|base| base.provenance.as_ref())
        .flat_map(|provenance| provenance.values().flatten())
        .filter_map(|evidence| evidence.key.as_deref())
        .collect();
    raw.iter()
        .filter(|(key, _)| !mapped.contains(key.as_str()))
        .map(|(key, values)| (key.clone(), values.clone()))
        .collect()
}

fn unresolved_credits(raw: &RawTagMap, path: &str) -> Vec<UnresolvedCredit> {
    raw.iter()
        .flat_map(|(key, values)| {
            values.iter().flat_map(move |value| {
                structured_credits(matching_key(key), value)
                    .into_iter()
                    .filter_map(move |credit| {
                        (canonical_roles(&credit.role).is_empty()
                            && explicit_instrument(key, &credit.role).is_none())
                        .then(|| UnresolvedCredit {
                            role: credit.role.into_owned(),
                            names: vec![credit.name.to_owned()],
                            source: FieldSource {
                                key: Some(key.clone()),
                                ..source(MetadataSourceKind::AudioTag, path, None)
                            },
                        })
                    })
            })
        })
        .collect()
}

fn identifiers(
    tags: &mut Tags<'_>,
    mapping: &[(&str, &[&str])],
) -> Option<crate::model::ExternalIdentifiers> {
    let identifiers: BTreeMap<_, _> = mapping
        .iter()
        .filter_map(|(namespace, keys)| {
            Some((
                (*namespace).to_owned(),
                tags.list(&format!("identifiers.{namespace}"), keys)?,
            ))
        })
        .collect();
    (!identifiers.is_empty()).then_some(identifiers)
}

fn links(tags: &mut Tags<'_>) -> Option<Vec<MetadataLink>> {
    use MetadataLinkRelation::{
        Artist, Copyright, Label, Official, Payment, Podcast, Purchase, Radio, Source,
    };
    let mapping: &[(MetadataLinkRelation, &[&str])] = &[
        (Official, &["URL", "URLOFFICIAL"]),
        (Artist, &["URLARTIST", "ARTISTWEBPAGE"]),
        (Label, &["URLLABEL", "PUBLISHERWEBPAGE"]),
        (Copyright, &["URLCOPYRIGHT", "COPYRIGHTURL"]),
        (Source, &["URLSOURCE", "AUDIOSOURCEWEBPAGE"]),
        (Purchase, &["URLPURCHASE"]),
        (Payment, &["URLPAYMENT", "PAYMENTWEBPAGE"]),
        (Podcast, &["URLPODCAST", "PODCASTURL"]),
        (Radio, &["URLINTERNETRADIO", "RADIOSTATIONWEBPAGE"]),
    ];
    let mut links = Vec::new();
    for (relation, keys) in mapping {
        for url in tags.list("links", keys).into_iter().flatten() {
            links.push(MetadataLink {
                relation: *relation,
                url,
                title: None,
            });
        }
    }
    (!links.is_empty()).then_some(links)
}
