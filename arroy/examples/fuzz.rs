use std::time::{Duration, Instant};
use std::{fmt, panic};

use arbitrary::{Arbitrary, Unstructured};
use arroy::distances::Euclidean;
use arroy::{Database, Reader, Result, Writer};
use heed::EnvOpenOptions;
use rand::rngs::StdRng;
use rand::{Fill, SeedableRng};

const TWENTY_GIB: usize = 20 * 1024 * 1024 * 1024;

const UPDATE_PER_BATCHES: usize = 50;
const NB_BATCHES: usize = 5;
const NB_DIFFERENT_VECTORS: u32 = 5;

#[derive(Clone)]
struct Document {
    id: u32,
    vec: Vec<f32>,
}

impl fmt::Debug for Document {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id.to_string())
    }
}

impl<'a> Arbitrary<'a> for Document {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let id = *u.choose(&(0..NB_DIFFERENT_VECTORS).collect::<Vec<_>>())?;
        Ok(Document { id, vec: vec![id as f32, 0.0] })
    }
}

#[derive(Debug, Clone, Arbitrary)]
enum Operation {
    Add(Document),
    Delete(Document),
}

fn main() -> Result<()> {
    let timer = std::env::args()
        .nth(1)
        .map(|s| Duration::from_secs(s.parse().expect("Expected a whole number of seconds")));

    let dir = tempfile::tempdir().unwrap();
    let env = unsafe { EnvOpenOptions::new().map_size(TWENTY_GIB).open(dir.path()) }?;
    let mut wtxn = env.write_txn()?;
    let database: Database<Euclidean> = env.create_database(&mut wtxn, None)?;
    wtxn.commit()?;

    let mut rng_points = StdRng::seed_from_u64(42);
    let rng_arroy = rng_points.clone();

    let total_duration = Instant::now();
    let mut instant = Instant::now();
    let mut smol_iterations = 0;

    for iteration in 0.. {
        // logging progression
        smol_iterations += 1;
        let elapsed = instant.elapsed();
        if elapsed >= Duration::from_secs(1) {
            println!(
                "Ran {smol_iterations} iterations in {elapsed:.1?} for a grand total of {iteration} iterations"
            );
            instant = Instant::now();
            smol_iterations = 0;

            if timer.map_or(false, |duration| duration < total_duration.elapsed()) {
                return Ok(());
            }
        }

        let mut v = [0_u8; 10_000];
        v.try_fill(&mut rng_points).unwrap();

        let mut data = Unstructured::new(&v);
        let batches =
            <[[Operation; UPDATE_PER_BATCHES]; NB_BATCHES]>::arbitrary(&mut data).unwrap();

        for operations in batches {
            let ops = operations.clone();
            let ret = panic::catch_unwind(|| -> arroy::Result<()> {
                let mut rng_arroy = rng_arroy.clone();
                let mut wtxn = env.write_txn()?;
                let writer = Writer::<Euclidean>::new(database, 0, 2);

                for op in operations {
                    match op {
                        Operation::Add(doc) => writer.add_item(&mut wtxn, doc.id, &doc.vec)?,
                        Operation::Delete(doc) => drop(writer.del_item(&mut wtxn, doc.id)?),
                    }
                }
                writer.builder(&mut rng_arroy).build(&mut wtxn)?;
                wtxn.commit()?;
                let rtxn = env.read_txn()?;
                let reader = Reader::<Euclidean>::open(&rtxn, 0, database)?;
                reader.assert_validity(&rtxn).unwrap();
                Ok(())
            });
            if let Err(e) = ret {
                #[cfg(feature = "plot")]
                {
                    let mut buffer = Vec::new();

                    let rtxn = env.read_txn()?;
                    let reader = Reader::<Euclidean>::open(&rtxn, 0, database)?;
                    reader.plot_internals_tree_nodes(&rtxn, &mut buffer)?;
                    std::fs::write("plot.dot", &buffer).unwrap();
                    println!("Plotted your database to `plot.dot`");
                }
                dbg!(&ops);
                dbg!(e);
                return Ok(());
            }
        }
    }

    Ok(())
}
