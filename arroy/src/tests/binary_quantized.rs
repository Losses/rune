use crate::{
    distance::BinaryQuantizedEuclidean,
    tests::{create_database, rng},
    Writer,
};

#[test]
fn write_and_retrieve_binary_quantized_vector() {
    let handle = create_database::<BinaryQuantizedEuclidean>();
    let mut wtxn = handle.env.write_txn().unwrap();
    let writer = Writer::new(handle.database, 0, 16);
    writer
        .add_item(
            &mut wtxn,
            0,
            &[
                -2.0, -1.0, 0.0, -0.1, 2.0, 2.0, -12.4, 21.2, -2.0, -1.0, 0.0, 1.0, 2.0, 2.0,
                -12.4, 21.2,
            ],
        )
        .unwrap();
    let vec = writer.item_vector(&wtxn, 0).unwrap().unwrap();
    insta::assert_debug_snapshot!(vec, @r###"
    [
        -1.0,
        -1.0,
        1.0,
        -1.0,
        1.0,
        1.0,
        -1.0,
        1.0,
        -1.0,
        -1.0,
        1.0,
        1.0,
        1.0,
        1.0,
        -1.0,
        1.0,
    ]
    "###);

    writer.builder(&mut rng()).n_trees(1).build(&mut wtxn).unwrap();
    wtxn.commit().unwrap();

    insta::assert_snapshot!(handle, @r#"
    ==================
    Dumping index 0
    Root: Metadata { dimensions: 16, items: RoaringBitmap<[0]>, roots: [0], distance: "binary quantized euclidean" }
    Version: Version { major: 0, minor: 6, patch: 2 }
    Tree 0: Descendants(Descendants { descendants: [0] })
    Item 0: Leaf(Leaf { header: NodeHeaderBinaryQuantizedEuclidean { bias: 0.0 }, vector: [-1.0000, -1.0000, 1.0000, -1.0000, 1.0000, 1.0000, -1.0000, 1.0000, -1.0000, -1.0000, "other ..."] })
    "#);
}
