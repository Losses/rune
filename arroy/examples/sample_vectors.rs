use std::io::{BufRead, BufReader};

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::SeedableRng;

const DEFAULT_COUNT: usize = 1_000_000;

fn main() {
    let count = std::env::args().nth(1).map(|c| c.parse().unwrap()).unwrap_or(DEFAULT_COUNT);
    let reader = BufReader::new(std::io::stdin());
    let mut vectors: Vec<(u32, Vec<f32>)> = Vec::new();

    for line in reader.lines() {
        let line = line.unwrap();
        if line.starts_with("===") {
            continue;
        }

        let (id, vector) = line.split_once(',').expect(&line);
        let id: u32 = id.parse().unwrap();

        let vector = vector
            .trim_matches(|c: char| c.is_whitespace() || c == '[' || c == ']')
            .split(',')
            .map(|s| s.trim().parse::<f32>().unwrap())
            .collect();

        vectors.push((id, vector));
        assert_eq!(vectors[0].1.len(), vectors.last().unwrap().1.len());
    }

    println!("=== BEGIN vectors ===");
    let vector_len = vectors[0].1.len();
    let mut rng = StdRng::seed_from_u64(42);
    let mut id = 0;
    for _ in 0..(count.checked_div(2).unwrap()) {
        let mut iter = vectors.choose_multiple(&mut rng, 2);
        let a = iter.next().unwrap();
        let b = iter.next().unwrap();

        let (al, ar) = a.1.split_at(vector_len / 2);
        let (bl, br) = b.1.split_at(vector_len - al.len()); // even numbers

        let newa: Vec<f32> = al.iter().copied().chain(br.iter().copied()).collect();
        let newb: Vec<f32> = bl.iter().copied().chain(ar.iter().copied()).collect();
        println!("{}, {:?}", id, newa);
        println!("{}, {:?}", id + 1, newb);

        id += 2;
    }
    println!("=== END vectors ===");
}
