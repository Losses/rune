use std::io::{ErrorKind, Read, Seek, SeekFrom, Write};
use std::num::NonZeroUsize;

use super::StorageProvider;
use super::bounded::{BoundedStorageProvider, BoundedStorageReader, BoundedStorageWriter};
use super::memory::{MemoryStorage, MemoryStorageProvider};

type Reader = BoundedStorageReader<MemoryStorage>;
type Writer = BoundedStorageWriter<MemoryStorage>;

fn bounded(size: usize) -> (Reader, Writer) {
    let size = NonZeroUsize::new(size).expect("nonzero size");
    BoundedStorageProvider::new(MemoryStorageProvider, size)
        .into_reader_writer(None)
        .expect("bounded storage")
}

fn read_some(reader: &mut Reader, len: usize) -> Vec<u8> {
    let mut buf = vec![0xff; len];
    let read = reader.read(&mut buf).expect("read");
    buf.truncate(read);
    buf
}

#[test]
fn a_reader_seeks_forward_and_back_before_any_transfer() {
    let (mut reader, mut writer) = bounded(16);

    assert_eq!(reader.seek(SeekFrom::Start(8)).expect("seek forward"), 8);
    assert_eq!(reader.seek(SeekFrom::Start(0)).expect("seek back"), 0);

    writer.write_all(b"abcd").expect("write");
    assert_eq!(read_some(&mut reader, 8), b"abcd");
    assert_eq!(read_some(&mut reader, 8), b"");
}

#[test]
fn a_writer_seeks_forward_and_back_before_any_transfer() {
    let (mut reader, mut writer) = bounded(16);

    assert_eq!(writer.seek(SeekFrom::Start(8)).expect("seek forward"), 8);
    assert_eq!(writer.seek(SeekFrom::Start(0)).expect("seek back"), 0);

    writer.write_all(b"abcd").expect("write");
    assert_eq!(read_some(&mut reader, 8), b"abcd");
}

#[test]
fn seeks_back_past_the_transferred_count_after_transfers() {
    let (mut reader, mut writer) = bounded(16);
    writer.write_all(b"0123456789AB").expect("write");
    assert_eq!(read_some(&mut reader, 4), b"0123");

    reader
        .seek(SeekFrom::Start(10))
        .expect("reader seek forward");
    reader.seek(SeekFrom::Start(2)).expect("reader seek back");
    assert_eq!(read_some(&mut reader, 8), b"23456789");

    writer
        .seek(SeekFrom::Start(30))
        .expect("writer seek forward");
    writer.seek(SeekFrom::Start(12)).expect("writer seek back");
    writer.write_all(b"CD").expect("write after seeks");
    assert_eq!(read_some(&mut reader, 8), b"ABCD");
}

#[test]
fn a_reader_seeks_forward_and_back_across_the_ring_wrap() {
    let (mut reader, mut writer) = bounded(8);
    writer.write_all(b"abcdefgh").expect("fill the ring");
    assert_eq!(read_some(&mut reader, 6), b"abcdef");
    writer.write_all(b"ijklmn").expect("wrap the ring");

    reader.seek(SeekFrom::Start(13)).expect("seek forward");
    reader.seek(SeekFrom::Start(6)).expect("seek back");

    assert_eq!(read_some(&mut reader, 8), b"ghijklmn");
}

#[test]
fn a_writer_seeks_forward_and_back_across_the_ring_wrap() {
    let (mut reader, mut writer) = bounded(8);
    writer.write_all(b"abcdefgh").expect("fill the ring");
    assert_eq!(read_some(&mut reader, 6), b"abcdef");
    writer.write_all(b"ijklmn").expect("wrap the ring");
    assert_eq!(read_some(&mut reader, 8), b"ghijklmn");

    writer.seek(SeekFrom::Start(30)).expect("seek forward");
    writer.seek(SeekFrom::Start(14)).expect("seek back");
    writer.write_all(b"op").expect("write after seeks");

    assert_eq!(read_some(&mut reader, 8), b"op");
}

#[test]
fn a_seek_before_the_start_is_refused() {
    let (mut reader, mut writer) = bounded(16);

    let error = reader
        .seek(SeekFrom::Current(-1))
        .expect_err("a negative reader position is invalid");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    let error = writer
        .seek(SeekFrom::Current(-1))
        .expect_err("a negative writer position is invalid");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    let error = reader
        .seek(SeekFrom::End(0))
        .expect_err("seeking from the end is unsupported");
    assert_eq!(error.kind(), ErrorKind::Unsupported);

    writer.write_all(b"abcd").expect("write");
    assert_eq!(read_some(&mut reader, 8), b"abcd");
}

#[test]
fn a_writer_seek_ahead_makes_no_unwritten_bytes_readable() {
    let (mut reader, mut writer) = bounded(8);
    writer.write_all(b"abcd").expect("write");
    assert_eq!(read_some(&mut reader, 4), b"abcd");

    writer.seek(SeekFrom::Start(8)).expect("writer seek ahead");
    assert_eq!(read_some(&mut reader, 4), b"");
    assert_eq!(reader.stream_position().expect("reader position"), 4);

    writer.seek(SeekFrom::Start(4)).expect("writer seek back");
    writer.write_all(b"efgh").expect("write the skipped bytes");
    assert_eq!(read_some(&mut reader, 4), b"efgh");
}

#[test]
fn a_writer_seek_ahead_after_the_ring_wrap_makes_no_old_bytes_readable() {
    let (mut reader, mut writer) = bounded(8);
    writer.write_all(b"abcdefgh").expect("fill the ring");
    assert_eq!(read_some(&mut reader, 8), b"abcdefgh");
    writer.write_all(b"ijklmnop").expect("wrap the ring");
    assert_eq!(read_some(&mut reader, 8), b"ijklmnop");

    writer.seek(SeekFrom::Start(24)).expect("writer seek ahead");
    assert_eq!(read_some(&mut reader, 8), b"");
    assert_eq!(reader.stream_position().expect("reader position"), 16);
}

#[test]
fn a_rewrite_below_the_written_end_keeps_the_bytes_above_it_readable() {
    let (mut reader, mut writer) = bounded(64);
    writer.write_all(b"0123456789").expect("write the start");
    writer.seek(SeekFrom::Start(20)).expect("writer seek ahead");
    writer
        .write_all(b"KLMNOPQRST")
        .expect("write after the gap");
    writer.seek(SeekFrom::Start(10)).expect("writer seek back");
    writer.write_all(b"ABCDEFGHIJ").expect("fill the gap");
    writer
        .seek(SeekFrom::Start(30))
        .expect("writer seek to the next gap");

    reader.seek(SeekFrom::Start(20)).expect("reader seek");
    assert_eq!(read_some(&mut reader, 10), b"KLMNOPQRST");
    reader.seek(SeekFrom::Start(0)).expect("reader seek back");
    assert_eq!(
        read_some(&mut reader, 30),
        b"0123456789ABCDEFGHIJKLMNOPQRST"
    );
}

#[test]
fn a_writer_seek_back_after_the_ring_wrap_exposes_no_overwritten_bytes() {
    let (mut reader, mut writer) = bounded(8);
    writer.write_all(b"abcdefgh").expect("fill the ring");
    assert_eq!(read_some(&mut reader, 4), b"abcd");
    writer.write_all(b"ijkl").expect("wrap over abcd");

    reader.seek(SeekFrom::Start(0)).expect("reader seek back");
    writer.seek(SeekFrom::Start(8)).expect("writer seek back");
    let mut buf = [0; 8];
    let error = reader
        .read(&mut buf)
        .expect_err("abcd were overwritten by ijkl");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);

    reader
        .seek(SeekFrom::Start(4))
        .expect("reader seek to the oldest byte held");
    assert_eq!(read_some(&mut reader, 8), b"efghijkl");
}

#[test]
fn a_wrapped_read_works_while_the_writer_is_behind_the_reader() {
    let (mut reader, mut writer) = bounded(8);
    writer.write_all(b"abcdefgh").expect("fill the ring");
    assert_eq!(read_some(&mut reader, 4), b"abcd");
    writer.write_all(b"ijkl").expect("wrap the ring");

    writer
        .seek(SeekFrom::Start(2))
        .expect("writer seek far back");
    assert_eq!(read_some(&mut reader, 8), b"efghijkl");
}

#[test]
fn a_rewrite_before_the_bytes_the_ring_holds_starts_the_ring_there() {
    let (mut reader, mut writer) = bounded(8);
    writer.write_all(b"abcdefgh").expect("fill the ring");
    assert_eq!(read_some(&mut reader, 8), b"abcdefgh");
    writer.write_all(b"ijklmnop").expect("wrap the ring");
    assert_eq!(read_some(&mut reader, 8), b"ijklmnop");

    writer
        .seek(SeekFrom::Start(2))
        .expect("writer seek before the ring");
    writer
        .write_all(b"CD")
        .expect("rewrite 2..4 over the slots of 10..12");

    reader
        .seek(SeekFrom::Start(2))
        .expect("reader seek to the rewrite");
    assert_eq!(read_some(&mut reader, 8), b"CD");
    reader
        .seek(SeekFrom::Start(0))
        .expect("reader seek before the rewrite");
    let mut buf = [0; 2];
    let error = reader
        .read(&mut buf)
        .expect_err("the slots of 0..2 hold 8..10");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
}

#[test]
fn a_write_past_the_largest_position_is_refused() {
    let (mut reader, mut writer) = bounded(16);
    let last = (usize::MAX - 2) as u64;
    reader.seek(SeekFrom::Start(last)).expect("reader seek");
    writer.seek(SeekFrom::Start(last)).expect("writer seek");

    let error = writer
        .write(b"abcdefgh")
        .expect_err("eight bytes do not fit below usize::MAX");
    assert_eq!(error.kind(), ErrorKind::InvalidInput);
    assert_eq!(writer.stream_position().expect("writer position"), last);

    writer.write_all(b"ab").expect("two bytes fit");
    assert_eq!(read_some(&mut reader, 8), b"ab");
}

#[test]
fn a_writer_behind_the_reader_hands_out_none_of_the_bytes_it_overwrites() {
    let (mut reader, mut writer) = bounded(8);
    writer.write_all(b"abcdefgh").expect("fill the ring");
    assert_eq!(read_some(&mut reader, 4), b"abcd");
    writer.write_all(b"ijkl").expect("wrap over abcd");

    // The writer's free space is not limited by a reader it is behind, so filling the gap 2..4
    // reuses the slots of the unread 10..12. Those positions are no longer stored: the reader
    // gets the bytes still held and then nothing, never the bytes of 2..4.
    writer.seek(SeekFrom::Start(2)).expect("writer seek back");
    writer.write_all(b"CD").expect("write behind the reader");
    assert_eq!(read_some(&mut reader, 8), b"efghij");
    assert_eq!(
        read_some(&mut reader, 8),
        b"",
        "the slots of 10..12 hold 2..4"
    );
}

#[test]
fn a_jump_after_a_rewrite_reads_no_bytes_of_another_position() {
    let (mut reader, mut writer) = bounded(8);
    writer.write_all(b"abcdefgh").expect("fill the ring");
    assert_eq!(read_some(&mut reader, 8), b"abcdefgh");
    writer.write_all(b"ijklmnop").expect("wrap the ring");
    assert_eq!(read_some(&mut reader, 8), b"ijklmnop");

    writer.seek(SeekFrom::Start(2)).expect("writer seek back");
    writer
        .write_all(b"CD")
        .expect("rewrite 2..4 over the slots of 10..12");
    writer.seek(SeekFrom::Start(16)).expect("writer seek ahead");
    writer
        .write_all(b"qr")
        .expect("write 16..18 over the slots of 8..10");

    reader.seek(SeekFrom::Start(10)).expect("reader seek to 10");
    let mut buf = [0; 8];
    if let Ok(read) = reader.read(&mut buf) {
        assert_eq!(read, 0, "the slots of 10..12 hold 2..4: {:?}", &buf[..read]);
    }
    reader.seek(SeekFrom::Start(12)).expect("reader seek to 12");
    assert_eq!(read_some(&mut reader, 8), b"mnopqr");
}

#[test]
fn a_rewrite_split_into_pieces_keeps_what_one_write_keeps() {
    let one: [&[u8]; 1] = [b"EFGH"];
    let two: [&[u8]; 2] = [b"EF", b"GH"];
    for pieces in [&one[..], &two[..]] {
        let (mut reader, mut writer) = bounded(8);
        writer.write_all(b"abcdefgh").expect("fill the ring");
        assert_eq!(read_some(&mut reader, 8), b"abcdefgh");
        writer.write_all(b"ijklmnop").expect("wrap the ring");
        assert_eq!(read_some(&mut reader, 8), b"ijklmnop");

        writer.seek(SeekFrom::Start(4)).expect("writer seek back");
        for piece in pieces {
            writer.write_all(piece).expect("rewrite 4..8");
        }
        reader
            .seek(SeekFrom::Start(4))
            .expect("reader seek to the rewrite");
        assert_eq!(read_some(&mut reader, 8), b"EFGHijkl", "{pieces:?}");
    }
}

#[test]
fn a_rewrite_that_runs_into_the_bytes_the_ring_holds_joins_them() {
    let (mut reader, mut writer) = bounded(8);
    writer.write_all(b"abcdefgh").expect("fill the ring");
    assert_eq!(read_some(&mut reader, 8), b"abcdefgh");
    writer.write_all(b"ijklmnop").expect("wrap the ring");
    assert_eq!(read_some(&mut reader, 8), b"ijklmnop");

    writer.seek(SeekFrom::Start(4)).expect("writer seek back");
    writer
        .write_all(b"EFGHIJ")
        .expect("rewrite 4..10, through 8..10");
    reader
        .seek(SeekFrom::Start(4))
        .expect("reader seek to the rewrite");
    assert_eq!(read_some(&mut reader, 8), b"EFGHIJkl");
    let mut buf = [0; 2];
    if let Ok(read) = reader.read(&mut buf) {
        assert_eq!(read, 0, "the slots of 12..14 hold 4..6: {:?}", &buf[..read]);
    }
}
