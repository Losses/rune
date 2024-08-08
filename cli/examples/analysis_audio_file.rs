use analysis::analysis::{analyze_audio, normalize_analysis_result};

fn main() {
    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("file path not provided");

    // Process the audio file and perform FFT using Overlap-Save method.
    let analysis_result = normalize_analysis_result(analyze_audio(path, 4096, 4096 / 2));

    println!("{:#?}", analysis_result);
}
