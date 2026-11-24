# Chained Ogg test fixtures

These are single-stream Ogg Vorbis files. Concatenating two of them byte for byte makes a
valid chained Ogg file with two physical streams, which is what the decoder and chart
analyzer regression tests exercise.

- `tone_440_44100.ogg`: 2 s, 440 Hz, mono, 44100 Hz
- `tone_660_44100.ogg`: 2 s, 660 Hz, mono, 44100 Hz
- `tone_660_48000.ogg`: 2 s, 660 Hz, mono, 48000 Hz (an unsupported transition when
  appended to `tone_440_44100.ogg`)

They were generated with ffmpeg:

```sh
ffmpeg -f lavfi -i 'sine=frequency=440:duration=2:sample_rate=44100' \
  -ac 1 -c:a libvorbis -q:a 3 tone_440_44100.ogg
ffmpeg -f lavfi -i 'sine=frequency=660:duration=2:sample_rate=44100' \
  -ac 1 -c:a libvorbis -q:a 3 tone_660_44100.ogg
ffmpeg -f lavfi -i 'sine=frequency=660:duration=2:sample_rate=48000' \
  -ac 1 -c:a libvorbis -q:a 3 tone_660_48000.ogg
```

The tests concatenate them at run time, so the fixtures stay usable for single-stream and
unsupported-transition cases too.
