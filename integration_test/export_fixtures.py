"""Prepare 1001 short, distinctly tagged Ogg files before the app starts.

Usage: prepare(run_dir, music_dir, env, ffmpeg_path, vorbiscomment_path)
music_dir must be an existing, empty folder; the app later adds it as a library.
"""
import datetime
import hashlib
import json
import shutil
import subprocess


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def prepare(run, music, env, ffmpeg, vorbis):
    assert music.is_dir() and not any(music.iterdir())
    started = datetime.datetime.now(datetime.timezone.utc).isoformat()
    sources = run / 'fixture-sources'
    sources.mkdir()
    seed = sources / 'export-short-seed.ogg'
    command = [
        ffmpeg, '-nostdin', '-v', 'error', '-f', 'lavfi', '-i',
        'sine=frequency=440:sample_rate=48000:duration=0.25', '-ac', '1',
        '-c:a', 'libvorbis', '-q:a', '3', str(seed),
    ]
    generated = subprocess.run(
        command, env=env, capture_output=True, check=True, timeout=30
    )
    assert not generated.stdout and not generated.stderr
    decode_command = [
        ffmpeg, '-nostdin', '-v', 'error', '-xerror', '-i', str(seed),
        '-map', '0:a:0', '-f', 's16le', '-acodec', 'pcm_s16le', '-',
    ]
    decoded = subprocess.run(
        decode_command, env=env, capture_output=True, check=True, timeout=30
    )
    assert decoded.stdout and not decoded.stderr
    files = []
    for number in range(1, 1002):
        title = f'Export Track {number:04d}'
        audio = music / f'export-{number:04d}.ogg'
        shutil.copy2(seed, audio)
        tag_command = [
            vorbis, '-w', '-R', '-t', 'TITLE=' + title,
            '-t', 'ARTIST=Runtime Export', '-t', 'ALBUM=Owned Export Fixture',
            str(audio),
        ]
        tagged = subprocess.run(
            tag_command, env=env, capture_output=True, check=True, timeout=10
        )
        assert not tagged.stdout and not tagged.stderr
        files.append(
            dict(
                path=str(audio),
                relative_path=audio.name,
                title=title,
                size=audio.stat().st_size,
                sha256=sha(audio),
                tag_exit=tagged.returncode,
            )
        )
    (run / 'export-output').mkdir()
    receipt = dict(
        status='PREPARED_BEFORE_APP',
        started_utc=started,
        ended_utc=datetime.datetime.now(datetime.timezone.utc).isoformat(),
        target_dir=str(music),
        count=1001,
        files=files,
        total_bytes=sum(row['size'] for row in files),
    )
    with (run / 'export-fixtures.json').open('x') as stream:
        json.dump(receipt, stream, ensure_ascii=False, indent=2)
        stream.write('\n')
    return receipt
