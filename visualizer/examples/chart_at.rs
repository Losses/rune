//! What scenes would see from the chart of an audio file: `chart_at AUDIO [SECONDS...]`.
//! Analyses the file as the visualizer does when it plays, prints how long that took and the
//! chart's state at each time (default: every 2 s of the first half minute).

use std::sync::atomic::AtomicBool;
use std::time::Instant;

use fsio::FsIo;
use visualizer::Chart;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let audio = std::fs::canonicalize(args.next().expect("audio path"))?;
    let start = Instant::now();
    let analysis = chart_analyzer::analyze_file(&FsIo::new(), &audio, &AtomicBool::new(false))?;
    println!("{}: analysed in {:.2?}", audio.display(), start.elapsed());
    let chart = Chart::from_analysis(analysis);
    let mut times: Vec<f64> = args.map(|a| a.parse().expect("seconds")).collect();
    if times.is_empty() {
        times = (0..15).map(|i| f64::from(i) * 2.0).collect();
    }
    for t in times {
        println!("{t:6.2} {:?}", chart.at(t));
    }
    Ok(())
}
