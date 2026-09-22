//! Download the associated file at https://www.notion.so/meilisearch/Movies-embeddings-1de3258859f54b799b7883882219d266

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use arroy::distances::DotProduct;
use arroy::{Database, Writer};
use clap::Parser;
use heed::{EnvFlags, EnvOpenOptions};
use rand::rngs::StdRng;
use rand::SeedableRng;

/// 200 GiB
const DEFAULT_MAP_SIZE: usize = 1024 * 1024 * 1024 * 200;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Sets a custom database path.
    #[arg(default_value = "import.ary")]
    database: PathBuf,

    /// Specify the size of the database.
    #[arg(long, default_value_t = DEFAULT_MAP_SIZE)]
    map_size: usize,

    /// The number of dimensions to construct the arroy tree.
    #[arg(long, default_value_t = 768)]
    dimensions: usize,

    /// Use the MDB_WRITEMAP option to reduce the memory usage of LMDB.
    #[arg(long)]
    write_map: bool,

    /// Do not try to append items into the database.
    #[arg(long)]
    no_append: bool,

    /// The number of tress to generate.
    #[arg(long)]
    n_trees: Option<usize>,

    /// The seed to generate the internal trees.
    #[arg(long, default_value_t = 42)]
    seed: u64,
}

fn main() -> Result<(), heed::BoxedError> {
    env_logger::init();

    let Cli { database, map_size, dimensions, write_map, no_append, n_trees, seed } = Cli::parse();

    let mut rng = StdRng::seed_from_u64(seed);
    let reader = BufReader::new(std::io::stdin());

    let _ = fs::create_dir_all(&database);

    // Open the environment with the appropriate flags.
    let flags = if write_map { EnvFlags::WRITE_MAP } else { EnvFlags::empty() };
    let mut env_builder = EnvOpenOptions::new();
    env_builder.map_size(map_size);
    unsafe { env_builder.flags(flags) };
    let env = unsafe { env_builder.open(&database) }.unwrap();

    let mut wtxn = env.write_txn().unwrap();
    let database: Database<DotProduct> = env.create_database(&mut wtxn, None)?;
    let writer = Writer::<DotProduct>::new(database, 0, dimensions);

    // The file look like that
    // === BEGIN vectors ===
    // 0, [0.010056925, -0.0045358953, 0.009904552, 0.0046241777, ..., -0.050245073]
    // === END vectors ===

    let now = Instant::now();
    let mut insertion_time = Duration::default();
    let mut count = 0;
    for line in reader.lines() {
        let line = line?;
        if line.starts_with("===") {
            continue;
        }

        let (id, vector) = line.split_once(',').expect(&line);
        let id: u32 = id.parse()?;
        let vector: Vec<_> = vector
            .trim_matches(|c: char| c.is_whitespace() || c == '[' || c == ']')
            .split(',')
            .map(|s| s.trim().parse::<f32>().unwrap())
            .collect();

        let now = Instant::now();
        if no_append {
            writer.add_item(&mut wtxn, id, &vector)?;
        } else {
            writer.append_item(&mut wtxn, id, &vector)?;
        }
        insertion_time += now.elapsed();
        count += 1;
    }
    println!("Took {:.2?} to parse and insert into arroy", now.elapsed() - insertion_time);
    println!("Took {insertion_time:.2?} insert into arroy");
    println!("There are {count} vectors");
    println!();

    println!("Building the arroy internal trees...");
    let now = Instant::now();
    let mut builder = writer.builder(&mut rng);
    if let Some(n_trees) = n_trees {
        builder.n_trees(n_trees);
    }
    builder.build(&mut wtxn)?;
    wtxn.commit().unwrap();
    println!("Took {:.2?} to build", now.elapsed());

    Ok(())
}
