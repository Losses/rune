use analysis::{
    analysis::{analyze_audio, normalize_analysis_result},
    computing_device_type::ComputingDevice,
};

fn main() {
    // Get the first command line argument.
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).expect("file path not provided");

    let result = analyze_audio(path, 4096, 4096 / 2, ComputingDevice::Gpu, None);

    let analysis_result = match result {
        Ok(x) =>
        // Process the audio file and perform FFT using Overlap-Save method.
        {
            if let Some(x) = x {
                normalize_analysis_result(&x)
            } else {
                panic!("Analysis canceled");
            }
        }
        Err(e) => {
            println!("Error: {}", e);
            panic!("Unable to analysis the track");
        }
    };

    println!("{:#?}", analysis_result);
}
