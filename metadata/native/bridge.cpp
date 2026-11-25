#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <iterator>
#include <string>

#include "aifffile.h"
#include "aiffproperties.h"
#include "fileref.h"
#include "id3v2tag.h"
#include "flacproperties.h"
#include "mp4properties.h"
#include "mpegproperties.h"
#include "mpegfile.h"
#include "opusproperties.h"
#include "synchronizedlyricsframe.h"
#include "textidentificationframe.h"
#include "tpropertymap.h"
#include "tvariant.h"
#include "vorbisproperties.h"
#include "wavproperties.h"
#include "wavfile.h"

extern "C" {
struct RuneTagLibCallbacks {
  std::size_t (*read)(void *, char *, std::size_t);
  std::int64_t (*seek)(void *, std::int64_t, int);
  void (*tag)(void *, const char *, const char *, std::size_t);
  void (*picture)(void *, const char *, const char *, const char *, const char *, std::size_t);
};

struct RuneTagLibAudio {
  std::uint64_t frames;
  std::int32_t duration_ms, sample_rate, channels, bitrate_kbps, bits_per_sample;
  const char *format, *codec;
};
}

namespace {
// TagLib borrows this stream for the duration of the read.
class Stream final : public TagLib::IOStream {
public:
  Stream(void *context, const char *name, std::int64_t length, const RuneTagLibCallbacks &callbacks)
      : context_(context), name_(name), length_(length), callbacks_(callbacks) {}

  TagLib::FileName name() const override { return name_.c_str(); }
  bool readOnly() const override { return true; }
  bool isOpen() const override { return !failed_; }
  TagLib::offset_t tell() const override { return position_; }
  TagLib::offset_t length() override { return length_; }

  // Bound each allocation by the bytes still available in the source.
  TagLib::ByteVector readBlock(std::size_t length) override {
    if (failed_ || position_ >= length_) return {};
    length = static_cast<std::size_t>(std::min<std::uint64_t>(length, length_ - position_));
    TagLib::ByteVector bytes(length, 0);
    const auto read = callbacks_.read(context_, bytes.data(), length);
    position_ += read;
    bytes.resize(read);
    return bytes;
  }

  void seek(TagLib::offset_t offset, Position position) override {
    const auto result = callbacks_.seek(context_, offset, static_cast<int>(position));
    if (result < 0) {
      failed_ = true;
      return;
    }
    position_ = result;
  }

  // Rune opens files for reading through this bridge.
  void writeBlock(const TagLib::ByteVector &) override {}
  void insert(const TagLib::ByteVector &, TagLib::offset_t, std::size_t) override {}
  void removeBlock(TagLib::offset_t, std::size_t) override {}
  void truncate(TagLib::offset_t) override {}

private:
  void *context_;
  std::string name_;
  std::int64_t length_, position_ = 0;
  const RuneTagLibCallbacks &callbacks_;
  bool failed_ = false;
};

void audio_properties(const TagLib::AudioProperties &props, RuneTagLibAudio &out) {
  out = {0, props.lengthInMilliseconds(), props.sampleRate(), props.channels(), props.bitrate(),
         0, nullptr, nullptr};
  if (const auto *flac = dynamic_cast<const TagLib::FLAC::Properties *>(&props)) {
    out.format = out.codec = "flac";
    out.frames = flac->sampleFrames();
    out.bits_per_sample = flac->bitsPerSample();
  } else if (const auto *mpeg = dynamic_cast<const TagLib::MPEG::Properties *>(&props)) {
    out.format = "mpeg";
    out.codec = mpeg->layer() == 3 ? "mp3" : mpeg->layer() == 2 ? "mp2" : "mp1";
  } else if (const auto *mp4 = dynamic_cast<const TagLib::MP4::Properties *>(&props)) {
    out.format = "mp4";
    if (mp4->codec() == TagLib::MP4::Properties::AAC) out.codec = "aac";
    if (mp4->codec() == TagLib::MP4::Properties::ALAC) out.codec = "alac";
    out.bits_per_sample = mp4->bitsPerSample();
  } else if (dynamic_cast<const TagLib::Ogg::Vorbis::Properties *>(&props)) {
    out.format = "ogg";
    out.codec = "vorbis";
  } else if (dynamic_cast<const TagLib::Ogg::Opus::Properties *>(&props)) {
    out.format = "ogg";
    out.codec = "opus";
  } else if (const auto *wav = dynamic_cast<const TagLib::RIFF::WAV::Properties *>(&props)) {
    out.format = "wav";
    if (wav->format() == 1 || wav->format() == 3) out.codec = "pcm";
    out.frames = wav->sampleFrames();
    out.bits_per_sample = wav->bitsPerSample();
  } else if (const auto *aiff = dynamic_cast<const TagLib::RIFF::AIFF::Properties *>(&props)) {
    out.format = "aiff";
    out.frames = aiff->sampleFrames();
    out.bits_per_sample = aiff->bitsPerSample();
  }
}

void emit_pictures(TagLib::FileRef &file, void *context, const RuneTagLibCallbacks &callbacks) {
  for (const auto &picture : file.complexProperties("PICTURE")) {
    const auto mime = picture.value("mimeType").toString().to8Bit(true);
    const auto description = picture.value("description").toString().to8Bit(true);
    const auto kind = picture.value("pictureType").toString().to8Bit(true);
    const auto data = picture.value("data").toByteVector();
    callbacks.picture(context, mime.c_str(), description.c_str(), kind.c_str(), data.data(), data.size());
  }
}

const TagLib::ID3v2::Tag *id3_tag(TagLib::File *file) {
  if (auto *mpeg = dynamic_cast<TagLib::MPEG::File *>(file)) return mpeg->ID3v2Tag();
  if (auto *wav = dynamic_cast<TagLib::RIFF::WAV::File *>(file)) return wav->ID3v2Tag();
  if (auto *aiff = dynamic_cast<TagLib::RIFF::AIFF::File *>(file)) return aiff->tag();
  return nullptr;
}

void emit_tag(void *context, const RuneTagLibCallbacks &callbacks, const char *key,
              const std::string &value) {
  callbacks.tag(context, key, value.data(), value.size());
}

void emit_native_id3(const TagLib::ID3v2::Tag &tag, void *context, const RuneTagLibCallbacks &callbacks) {
  // Preserve native disc subtitle and date keys for precedence and tracing.
  for (const auto &key : {"TSST", "TDRC", "TDRL", "TDOR"}) {
    for (const auto *frame : tag.frameList(key)) {
      if (const auto *text = dynamic_cast<const TagLib::ID3v2::TextIdentificationFrame *>(frame))
        for (const auto &value : text->fieldList())
          emit_tag(context, callbacks, key, value.to8Bit(true));
    }
  }
  for (const auto *frame : tag.frameList("TXXX")) {
    const auto *text = dynamic_cast<const TagLib::ID3v2::UserTextIdentificationFrame *>(frame);
    if (!text || text->description().upper() != "DISCSUBTITLE") continue;
    const auto values = text->fieldList();
    if (values.isEmpty()) continue;
    for (auto value = std::next(values.begin()); value != values.end(); ++value)
      emit_tag(context, callbacks, "TXXX:DISCSUBTITLE", value->to8Bit(true));
  }

  // The normalized property map drops involved-people frames with unfamiliar role names.
  for (const auto &key : {"TIPL", "TMCL", "IPLS"}) {
    for (const auto *frame : tag.frameList(key)) {
      const auto *text = dynamic_cast<const TagLib::ID3v2::TextIdentificationFrame *>(frame);
      if (!text) continue;
      const auto fields = text->fieldList();
      if (fields.size() % 2 != 0) {
        for (const auto &value : fields) emit_tag(context, callbacks, key, value.to8Bit(true));
        continue;
      }
      for (auto entry = fields.begin(); entry != fields.end(); ++entry) {
        const auto name = std::string(key) + ":" + entry->to8Bit(true);
        emit_tag(context, callbacks, name.c_str(), (++entry)->to8Bit(true));
      }
    }
  }

  // Convert synchronized lyrics with millisecond timestamps to LRC.
  for (const auto *frame : tag.frameList("SYLT")) {
    using Lyrics = TagLib::ID3v2::SynchronizedLyricsFrame;
    const auto *lyrics = dynamic_cast<const Lyrics *>(frame);
    if (!lyrics || lyrics->type() != Lyrics::Lyrics ||
        lyrics->timestampFormat() != Lyrics::AbsoluteMilliseconds) continue;
    std::string lrc;
    for (const auto &line : lyrics->synchedText()) {
      char timestamp[32];
      std::snprintf(timestamp, sizeof(timestamp), "[%02u:%02u.%03u]", line.time / 60000,
                    line.time / 1000 % 60, line.time % 1000);
      lrc += timestamp + line.text.to8Bit(true) + "\n";
    }
    if (!lrc.empty()) emit_tag(context, callbacks, "SYLT", lrc);
  }
}
}

extern "C" int rune_taglib_read(void *context, const char *name, std::int64_t length,
                                 const RuneTagLibCallbacks *callbacks, RuneTagLibAudio *out) noexcept {
  try {
    Stream stream(context, name, length, *callbacks);
    TagLib::FileRef file(&stream);
    if (file.isNull() || !file.file()->isValid()) return 0;
    if (const auto *props = file.audioProperties()) audio_properties(*props, *out);

    // UTF-8 strings and picture buffers are borrowed only during each callback.
    if (callbacks->tag) {
      const auto *id3 = id3_tag(file.file());
      for (const auto &[key, values] : file.properties()) {
        if (id3 && key == "DISCSUBTITLE" && !id3->frameList("TSST").isEmpty()) continue;
        const auto native_key = key.to8Bit(true);
        for (const auto &value : values)
          emit_tag(context, *callbacks, native_key.c_str(), value.to8Bit(true));
      }
      if (id3) emit_native_id3(*id3, context, *callbacks);
    }
    if (callbacks->picture) emit_pictures(file, context, *callbacks);
    return stream.isOpen() ? 1 : 0;
  } catch (...) {
    return 0;
  }
}
