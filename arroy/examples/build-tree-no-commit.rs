//! This is a special version that do not commit to let us compute the trees without having to import the vectors again.
//!
//! Download the associated file at https://www.notion.so/meilisearch/Movies-embeddings-1de3258859f54b799b7883882219d266

use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use arroy::distances::DotProduct;
use arroy::{Database, Reader, Stats, TreeStats, Writer};
use clap::Parser;
use heed::{EnvFlags, EnvOpenOptions};
use rand::rngs::StdRng;
use rand::SeedableRng;

/// 2 GiB
const DEFAULT_MAP_SIZE: usize = 1024 * 1024 * 1024 * 2;

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

    /// The number of tress to generate.
    #[arg(long)]
    n_trees: Option<usize>,

    /// The seed to generate the internal trees.
    #[arg(long, default_value_t = 42)]
    seed: u64,
}

fn main() -> Result<(), heed::BoxedError> {
    env_logger::init();

    let Cli { database, map_size, dimensions, write_map, n_trees, seed } = Cli::parse();

    // Open the environment with the appropriate flags.
    let _ = fs::create_dir_all(&database);
    let flags = if write_map { EnvFlags::WRITE_MAP } else { EnvFlags::empty() };
    let mut env_builder = EnvOpenOptions::new();
    env_builder.map_size(map_size);
    unsafe { env_builder.flags(flags) };
    let env = unsafe { env_builder.open(&database) }.unwrap();

    let mut wtxn = env.write_txn().unwrap();
    let database: Database<DotProduct> = env.create_database(&mut wtxn, None)?;
    let writer = Writer::<DotProduct>::new(database, 0, dimensions);

    let now = Instant::now();
    println!("Building the arroy internal trees...");
    let mut rng = StdRng::seed_from_u64(seed);
    let mut builder = writer.builder(&mut rng);
    if let Some(n_trees) = n_trees {
        builder.n_trees(n_trees);
    }
    builder.build(&mut wtxn).unwrap();
    println!("Took {:.2?} to build", now.elapsed());

    let reader = Reader::open(&wtxn, 0, database)?;

    let mut dummy_sum = 0;
    let mut depth_sum = 0;
    let mut split_nodes_sum = 0;
    let mut descendants_sum = 0;

    let Stats { tree_stats, leaf } = reader.stats(&wtxn)?;
    let nb_roots = tree_stats.len();
    println!("There are {nb_roots} trees in this arroy index for a total of {leaf} leaf.");

    #[allow(clippy::unused_enumerate_index)]
    for (_i, TreeStats { depth, dummy_normals, split_nodes, descendants }) in
        tree_stats.into_iter().enumerate()
    {
        depth_sum += depth;
        dummy_sum += dummy_normals;
        split_nodes_sum += split_nodes;
        descendants_sum += descendants;

        // println!("Tree {_i} as a depth of {depth}, {split_nodes} split nodes, {dummy_normals} dummy normals ({}%), and {descendants} descendants.", dummy_normals as f64 / split_nodes as f64 * 100.);
    }

    println!();
    println!("Over all the trees, on average:");
    println!("\tdepth:\t\t\t{:.2}", depth_sum as f64 / nb_roots as f64);
    println!("\tsplit nodes:\t\t{:.2}", split_nodes_sum as f64 / nb_roots as f64);
    println!(
        "\tdummy split nodes:\t{:.2} ({:.2}%)",
        dummy_sum as f64 / nb_roots as f64,
        dummy_sum as f64 / split_nodes_sum as f64 * 100.
    );
    println!("\tdescendants:\t\t{:.2}", descendants_sum as f64 / nb_roots as f64);
    println!();
    println!(
        "That makes a total of: {} tree nodes. {:.2}% of all the nodes",
        split_nodes_sum + descendants_sum,
        (split_nodes_sum + descendants_sum) as f64
            / (split_nodes_sum as u64 + descendants_sum as u64 + leaf) as f64
            * 100.,
    );

    Ok(())
}
