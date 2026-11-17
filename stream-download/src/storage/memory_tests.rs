use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};

use super::StorageProvider;
use super::memory::{MemoryStorage, MemoryStorageProvider};

fn preallocated(content_length: u64) -> (MemoryStorage, MemoryStorage) {
    MemoryStorageProvider
        .into_reader_writer(Some(content_length))
        .expect("memory storage")
}

fn read_some(reader: &mut MemoryStorage, len: usize) -> Vec<u8> {
    let mut buf = vec![0xff; len];
    let read = reader.read(&mut buf).expect("read");
    buf.truncate(read);
    buf
}

#[test]
fn a_read_after_a_seek_stops_at_the_end_of_a_partial_write() {
    let (mut reader, mut writer) = preallocated(16);
    writer.write_all(b"abcd").expect("write");

    reader.seek(SeekFrom::Start(2)).expect("seek");

    assert_eq!(read_some(&mut reader, 8), b"cd");
    assert_eq!(read_some(&mut reader, 8), b"");
}

#[test]
fn a_read_past_the_end_of_the_buffer_reads_nothing() {
    let (mut reader, mut writer) = preallocated(16);
    writer.write_all(b"abcd").expect("write");

    assert_eq!(reader.seek(SeekFrom::Start(32)).expect("seek"), 32);
    assert_eq!(read_some(&mut reader, 8), b"");

    reader.seek(SeekFrom::Start(0)).expect("seek back");
    assert_eq!(read_some(&mut reader, 8), b"abcd");
}

#[test]
fn reads_follow_the_ranges_a_seeking_writer_stored() {
    let (mut reader, mut writer) = preallocated(16);
    writer.write_all(b"abcd").expect("write");
    writer.seek(SeekFrom::Start(8)).expect("writer seek");
    writer.write_all(b"ijkl").expect("write after seek");

    assert_eq!(read_some(&mut reader, 16), b"abcd");
    assert_eq!(read_some(&mut reader, 16), b"", "the gap was never written");

    reader
        .seek(SeekFrom::Start(8))
        .expect("seek to the second range");
    assert_eq!(read_some(&mut reader, 16), b"ijkl");

    writer
        .seek(SeekFrom::Start(4))
        .expect("writer seek into the gap");
    writer.write_all(b"efgh").expect("fill the gap");
    reader.seek(SeekFrom::Start(2)).expect("seek");
    assert_eq!(read_some(&mut reader, 16), b"cdefghijkl");
}

#[test]
fn an_overwrite_does_not_extend_what_can_be_read() {
    let (mut reader, mut writer) = preallocated(16);
    writer.write_all(b"abcd").expect("write");
    writer.seek(SeekFrom::Start(0)).expect("writer seek");
    writer.write_all(b"AB").expect("overwrite");

    assert_eq!(read_some(&mut reader, 16), b"ABcd");
}

#[test]
fn a_seek_before_the_start_is_refused() {
    let (mut reader, mut writer) = preallocated(16);
    writer.write_all(b"abcd").expect("write");
    reader.seek(SeekFrom::Start(2)).expect("seek");

    let error = reader
        .seek(SeekFrom::Current(-3))
        .expect_err("a negative position is invalid");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    let error = reader
        .seek(SeekFrom::End(-17))
        .expect_err("a negative position is invalid");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);

    assert_eq!(
        read_some(&mut reader, 8),
        b"cd",
        "the position is unchanged"
    );
}

#[test]
fn sequential_reads_and_seeks_without_a_content_length() {
    let (mut reader, mut writer) = MemoryStorageProvider
        .into_reader_writer(None)
        .expect("memory storage");

    writer.write_all(b"abc").expect("write");
    assert_eq!(read_some(&mut reader, 8), b"abc");
    assert_eq!(read_some(&mut reader, 8), b"");

    writer.write_all(b"def").expect("write");
    assert_eq!(read_some(&mut reader, 2), b"de");
    assert_eq!(read_some(&mut reader, 8), b"f");

    assert_eq!(reader.seek(SeekFrom::End(-2)).expect("seek from end"), 4);
    assert_eq!(read_some(&mut reader, 8), b"ef");
    assert_eq!(reader.seek(SeekFrom::Current(-6)).expect("seek back"), 0);
    let mut all = Vec::new();
    reader.read_to_end(&mut all).expect("read to end");
    assert_eq!(all, b"abcdef");
}
