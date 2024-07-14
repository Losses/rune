use analysis::fft::fft;

fn main() {
    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("file path not provided");

    // Process the audio file and perform FFT using Overlap-Save method.
    let fft_results = fft(path, 4608, 2304);

    // Print the FFT results.
    for (i, fft_result) in fft_results.spectrum.iter().enumerate() {
        println!("{}: {:?}", i, fft_result);
    }
}
