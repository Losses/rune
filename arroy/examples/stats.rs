use std::fs;
use std::path::PathBuf;

use arroy::distances::DotProduct;
use arroy::{Database, Reader, Stats, TreeStats};
use clap::Parser;
use heed::EnvOpenOptions;

#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Sets a custom database path.
    #[arg(default_value = "import.ary")]
    database: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let Cli { database } = Cli::parse();

    let _ = fs::create_dir_all(&database);
    let env = unsafe { EnvOpenOptions::new().open(&database) }?;

    let rtxn = env.read_txn()?;
    let database: Database<DotProduct> = env.open_database(&rtxn, None)?.unwrap();
    let reader = Reader::open(&rtxn, 0, database)?;

    let mut dummy_sum = 0;
    let mut depth_sum = 0;
    let mut split_nodes_sum = 0;
    let mut descendants_sum = 0;

    let Stats { tree_stats, leaf } = reader.stats(&rtxn)?;
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
