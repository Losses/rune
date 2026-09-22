use std::fmt::Display;
use std::num::NonZeroUsize;

use roaring::RoaringBitmap;

use super::*;
use crate::distance::Cosine;
use crate::distances::{Euclidean, Manhattan};
use crate::{ItemId, Reader, Writer};

pub struct NnsRes(pub Option<Vec<(ItemId, f32)>>);

impl Display for NnsRes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0 {
            Some(ref vec) => {
                for (id, dist) in vec {
                    writeln!(f, "id({id}): distance({dist})")?;
                }
                Ok(())
            }
            None => f.write_str("No results found"),
        }
    }
}

#[test]
fn open_unfinished_db() {
    let handle = create_database();
    let mut wtxn = handle.env.write_txn().unwrap();
    let writer = Writer::new(handle.database, 0, 2);
    writer.add_item(&mut wtxn, 0, &[0.0, 0.0]).unwrap();
    wtxn.commit().unwrap();

    let rtxn = handle.env.read_txn().unwrap();
    let ret = Reader::<Euclidean>::open(&rtxn, 0, handle.database).map(|_| ()).unwrap_err();
    insta::assert_snapshot!(ret, @"Metadata are missing on index 0, You must build your database before attempting to read it");
}

#[test]
fn open_db_with_wrong_dimension() {
    let handle = create_database();
    let mut wtxn = handle.env.write_txn().unwrap();
    let writer = Writer::new(handle.database, 0, 2);
    writer.add_item(&mut wtxn, 0, &[0.0, 0.0]).unwrap();

    writer.builder(&mut rng()).n_trees(1).build(&mut wtxn).unwrap();
    wtxn.commit().unwrap();

    let rtxn = handle.env.read_txn().unwrap();
    let reader = Reader::<Euclidean>::open(&rtxn, 0, handle.database).unwrap();
    let ret = reader.nns(5).by_vector(&rtxn, &[1.0, 2.0, 3.0]).unwrap_err();
    insta::assert_snapshot!(ret, @"Invalid vector dimensions. Got 3 but expected 2");
}

#[test]
fn open_db_with_wrong_distance() {
    let handle = create_database::<Euclidean>();
    let mut wtxn = handle.env.write_txn().unwrap();
    let writer = Writer::new(handle.database, 0, 2);
    writer.add_item(&mut wtxn, 0, &[0.0, 0.0]).unwrap();

    writer.builder(&mut rng()).n_trees(1).build(&mut wtxn).unwrap();
    wtxn.commit().unwrap();

    let rtxn = handle.env.read_txn().unwrap();
    let wrongly_typed_db = handle.database.remap_data_type::<NodeCodec<Manhattan>>();
    let err = Reader::open(&rtxn, 0, wrongly_typed_db).unwrap_err();
    insta::assert_debug_snapshot!(err, @r###"
    UnmatchingDistance {
        expected: "euclidean",
        received: "manhattan",
    }
    "###);
}

#[test]
fn search_in_db_with_a_single_vector() {
    // https://github.com/meilisearch/meilisearch/pull/4296
    let handle = create_database::<Cosine>();
    let mut wtxn = handle.env.write_txn().unwrap();
    let writer = Writer::new(handle.database, 0, 3);
    writer.add_item(&mut wtxn, 0, &[0.00397, 0.553, 0.0]).unwrap();

    writer.builder(&mut rng()).build(&mut wtxn).unwrap();
    wtxn.commit().unwrap();

    let rtxn = handle.env.read_txn().unwrap();
    let reader = Reader::<Cosine>::open(&rtxn, 0, handle.database).unwrap();

    let ret = reader.nns(1).by_item(&rtxn, 0).unwrap();
    insta::assert_snapshot!(NnsRes(ret), @r###"
    id(0): distance(0)
    "###);
}

#[test]
fn two_dimension_on_a_line() {
    let handle = create_database();
    let mut wtxn = handle.env.write_txn().unwrap();
    let writer = Writer::new(handle.database, 0, 2);
    // We'll draw a simple line over the y as seen below
    // (0,0) # # # # # # ...
    for i in 0..100 {
        writer.add_item(&mut wtxn, i, &[i as f32, 0.0]).unwrap();
    }

    writer.builder(&mut rng()).n_trees(50).build(&mut wtxn).unwrap();
    wtxn.commit().unwrap();

    let rtxn = handle.env.read_txn().unwrap();
    let reader = Reader::<Euclidean>::open(&rtxn, 0, handle.database).unwrap();

    // if we can't look into enough nodes we find some random points
    let ret = reader.nns(5).search_k(NonZeroUsize::new(1).unwrap()).by_item(&rtxn, 1).unwrap();
    insta::assert_snapshot!(NnsRes(ret), @r###"
    id(48): distance(47)
    id(92): distance(91)
    "###);

    // if we can look into all the node there is no inifinite loop and it works
    let ret =
        reader.nns(5).search_k(NonZeroUsize::new(usize::MAX).unwrap()).by_item(&rtxn, 0).unwrap();
    insta::assert_snapshot!(NnsRes(ret), @r###"
    id(0): distance(0)
    id(1): distance(1)
    id(2): distance(2)
    id(3): distance(3)
    id(4): distance(4)
    "###);

    let ret = reader.nns(5).by_item(&rtxn, 0).unwrap();
    insta::assert_snapshot!(NnsRes(ret), @r###"
    id(0): distance(0)
    id(1): distance(1)
    id(2): distance(2)
    id(3): distance(3)
    id(4): distance(4)
    "###);
}

#[test]
fn two_dimension_on_a_column() {
    let handle = create_database();
    let mut wtxn = handle.env.write_txn().unwrap();
    let writer = Writer::new(handle.database, 0, 2);
    // We'll draw a simple line over the y as seen below
    // (0,0) # . . . . .
    // (0,1) # . . . . .
    // (0,2) # . . . . .
    // (0,3) # . . . . .
    // [...]
    for i in 0..100 {
        writer.add_item(&mut wtxn, i, &[0.0, i as f32]).unwrap();
    }

    writer.builder(&mut rng()).n_trees(50).build(&mut wtxn).unwrap();
    wtxn.commit().unwrap();

    let rtxn = handle.env.read_txn().unwrap();
    let reader = Reader::<Euclidean>::open(&rtxn, 0, handle.database).unwrap();
    let ret = reader.nns(5).by_item(&rtxn, 0).unwrap();

    insta::assert_snapshot!(NnsRes(ret), @r###"
    id(0): distance(0)
    id(1): distance(1)
    id(2): distance(2)
    id(3): distance(3)
    id(4): distance(4)
    "###);
}

#[test]
fn get_item_ids() {
    let handle = create_database();
    let mut wtxn = handle.env.write_txn().unwrap();
    let writer = Writer::new(handle.database, 0, 2);
    for i in 0..10 {
        writer.add_item(&mut wtxn, i, &[0.0, i as f32]).unwrap();
    }

    writer.builder(&mut rng()).n_trees(50).build(&mut wtxn).unwrap();

    let reader = Reader::<Euclidean>::open(&wtxn, 0, handle.database).unwrap();
    let ret = reader.item_ids();

    insta::assert_debug_snapshot!(ret, @"RoaringBitmap<[0, 1, 2, 3, 4, 5, 6, 7, 8, 9]>");
}

#[test]
fn filtering() {
    let handle = create_database();
    let mut wtxn = handle.env.write_txn().unwrap();
    let writer = Writer::new(handle.database, 0, 2);
    // We'll draw a simple line over the y as seen below
    // (0,0) # . . . . .
    // (0,1) # . . . . .
    // (0,2) # . . . . .
    // (0,3) # . . . . .
    // [...]
    for i in 0..100 {
        writer.add_item(&mut wtxn, i, &[0.0, i as f32]).unwrap();
    }

    writer.builder(&mut rng()).n_trees(50).build(&mut wtxn).unwrap();
    wtxn.commit().unwrap();

    let rtxn = handle.env.read_txn().unwrap();
    let reader = Reader::<Euclidean>::open(&rtxn, 0, handle.database).unwrap();

    let ret = reader.nns(5).candidates(&RoaringBitmap::from_iter(0..2)).by_item(&rtxn, 0).unwrap();
    insta::assert_snapshot!(NnsRes(ret), @r###"
    id(0): distance(0)
    id(1): distance(1)
    "###);

    let ret =
        reader.nns(5).candidates(&RoaringBitmap::from_iter(98..1000)).by_item(&rtxn, 0).unwrap();
    insta::assert_snapshot!(NnsRes(ret), @r###"
    id(98): distance(98)
    id(99): distance(99)
    "###);
}

#[test]
fn search_in_empty_database() {
    // See https://github.com/meilisearch/arroy/issues/75
    let handle = create_database::<Euclidean>();

    let mut wtxn = handle.env.write_txn().unwrap();
    let writer = Writer::new(handle.database, 0, 2);
    writer.builder(&mut rng()).build(&mut wtxn).unwrap();
    wtxn.commit().unwrap();

    let rtxn = handle.env.read_txn().unwrap();
    let reader = Reader::open(&rtxn, 0, handle.database).unwrap();
    let ret = reader.nns(10).by_vector(&rtxn, &[0., 0.]).unwrap();
    insta::assert_debug_snapshot!(ret, @"[]");
}

#[test]
fn try_reading_in_a_non_built_database() {
    // See https://github.com/meilisearch/arroy/issues/74
    let handle = create_database::<Euclidean>();

    let mut wtxn = handle.env.write_txn().unwrap();
    let writer = Writer::new(handle.database, 0, 2);
    writer.add_item(&mut wtxn, 0, &[0.0, 0.0]).unwrap();
    // We don't build the database
    wtxn.commit().unwrap();

    let rtxn = handle.env.read_txn().unwrap();
    let error = Reader::open(&rtxn, 0, handle.database).unwrap_err();
    insta::assert_debug_snapshot!(error, @r###"
    MissingMetadata(
        0,
    )
    "###);
    drop(rtxn);

    // we build the database once to get valid metadata
    let mut wtxn = handle.env.write_txn().unwrap();
    let writer = Writer::new(handle.database, 0, 2);
    writer.builder(&mut rng()).build(&mut wtxn).unwrap();
    let writer = Writer::new(handle.database, 0, 2);
    writer.del_item(&mut wtxn, 0).unwrap();
    // We don't build the database; this leaves the database in a corrupted state
    wtxn.commit().unwrap();

    let rtxn = handle.env.read_txn().unwrap();
    let error = Reader::open(&rtxn, 0, handle.database).unwrap_err();
    insta::assert_debug_snapshot!(error, @r###"
    NeedBuild(
        0,
    )
    "###);
}
