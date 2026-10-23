//! What presets would see from the chart of an audio file: `chart_at AUDIO [SECONDS...]`.
//! Looks the chart up as the visualizer does (chart folder, path key) and prints the chart's
//! state at each time (default: every 2 s of the first half minute).

use std::path::Path;

use visualizer::{Chart, chart_dir, chart_key};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let audio = std::fs::canonicalize(args.next().expect("audio path"))?;
    let file = chart_dir()
        .expect("no chart folder")
        .join(chart_key(Path::new(&audio)));
    println!("{}", file.display());
    let chart = Chart::load(&file)?;
    let mut times: Vec<f64> = args.map(|a| a.parse().expect("seconds")).collect();
    if times.is_empty() {
        times = (0..15).map(|i| f64::from(i) * 2.0).collect();
    }
    for t in times {
        println!("{t:6.2} {:?}", chart.at(t));
    }
    Ok(())
}
