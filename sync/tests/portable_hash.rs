use sync::protocol::PortableHasher;
#[test]
fn portable_blake3_matches_reference_across_resumes() {
    let sizes = [
        0usize, 1, 1023, 1024, 1025, 2047, 2048, 2049, 4095, 4096, 4097, 65535, 65536, 65537,
        1048699,
    ];
    for size in sizes {
        let input: Vec<u8> = (0..size)
            .map(|i| ((i.wrapping_mul(37) + 11) % 251) as u8)
            .collect();
        for seed in [1u64, 7, 99] {
            let mut rng = seed;
            let mut h = PortableHasher::new();
            let mut offset = 0;
            while offset < input.len() {
                rng = rng
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                let length = ((rng >> 32) as usize % 3000 + 1).min(input.len() - offset);
                h.update(&input[offset..offset + length]).unwrap();
                let serialized = serde_json::to_vec(&h).unwrap();
                h = serde_json::from_slice(&serialized).unwrap();
                offset += length;
            }
            assert_eq!(
                h.finish(),
                *blake3::hash(&input).as_bytes(),
                "size={size} seed={seed}"
            );
        }
    }
}
#[test]
fn portable_hash_checkpoint_rejects_malformed_fields() {
    let valid = PortableHasher::new().checkpoint();
    for bytes in [
        vec![],
        b"wrong".to_vec(),
        {
            let mut b = valid.clone();
            b[14] = 1;
            b
        },
        {
            let mut b = valid.clone();
            b[12] = 5;
            b
        },
        {
            let mut b = valid.clone();
            b[11] = 1;
            b
        },
    ] {
        assert!(PortableHasher::from_checkpoint(&bytes).is_err());
    }
}
