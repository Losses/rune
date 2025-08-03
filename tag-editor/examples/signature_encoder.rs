use clap::Parser;
use std::fs::File;
use std::io::{self, Read};

use tag_editor::shazam::spectrogram::{FrequencyPeak, Signature};

#[derive(Parser)]
struct Cli {
    /// Input JSON file containing the audio signature
    input: String,
}

fn main() -> io::Result<()> {
    let args = Cli::parse();

    let mut file = File::open(&args.input)?;
    let mut data = String::new();
    file.read_to_string(&mut data)?;

    let json_data: serde_json::Value = serde_json::from_str(&data).expect("Invalid JSON format");

    let sample_rate = json_data["sample_rate"].as_i64().unwrap() as i32;
    let num_samples = json_data["num_samples"].as_i64().unwrap() as i32;
    let peaks_by_band: [Vec<FrequencyPeak>; 5] = {
        let mut bands = [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for (i, band) in json_data["peaks_by_band"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            for peak in band.as_array().unwrap() {
                bands[i].push(FrequencyPeak {
                    pass: peak["pass"].as_i64().unwrap() as i32,
                    magnitude: peak["magnitude"].as_i64().unwrap() as i32,
                    bin: peak["bin"].as_i64().unwrap() as i32,
                });
            }
        }
        bands
    };

    let signature = Signature::new(sample_rate, num_samples, peaks_by_band);
    let binary_data = signature.encode();

    // Print as hex
    for (i, byte) in binary_data.iter().enumerate() {
        print!("{byte:02x}");
        if (i + 1) % 8 == 0 {
            println!();
        } else {
            print!(" ");
        }
    }
    if binary_data.len() % 8 != 0 {
        println!();
    }

    Ok(())
}
