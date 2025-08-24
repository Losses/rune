use std::sync::{Arc, Mutex};
use std::thread;

use rustfft::{FftPlanner, num_complex::Complex};

use simple_channel::{SimpleChannel, SimpleReceiver, SimpleSender};

pub struct RealTimeFFT {
    window_size: usize,
    window: Arc<Mutex<Vec<f32>>>,
    fft_window: Vec<f32>,
    fft_result_tx: SimpleSender<Vec<f32>>,
}

pub fn build_nuttall_window(window_size: usize) -> Vec<f32> {
    let a0 = 0.355768;
    let a1 = 0.487396;
    let a2 = 0.144232;
    let a3 = 0.012604;

    (0..window_size)
        .map(|n| {
            let factor = 2.0 * std::f32::consts::PI * n as f32 / (window_size as f32 - 1.0);
            a0 - a1 * factor.cos() + a2 * (2.0 * factor).cos() - a3 * (3.0 * factor).cos()
        })
        .collect()
}

impl RealTimeFFT {
    pub fn new(window_size: usize) -> Self {
        let (fft_result_tx, _) = SimpleChannel::channel(30);
        let window = vec![0.0; window_size];
        RealTimeFFT {
            window_size,
            window: Arc::new(Mutex::new(window)),
            fft_result_tx,
            fft_window: build_nuttall_window(window_size),
        }
    }

    pub fn add_data(&self, data: Vec<i16>) {
        let window_size = self.window_size;
        let mut window = self.window.lock().unwrap();

        // Calculate average value of data from all channels
        let avg: f32 = data.iter().map(|&x| x as f32).sum::<f32>() / data.len() as f32;

        // update the window
        window.push(avg);
        if window.len() > window_size {
            window.remove(0);
        }

        let fft_window = window.clone();
        let fft_result_tx = self.fft_result_tx.clone();
        let hanning_window = self.fft_window.clone();

        // Calculate the data in a new thread
        thread::spawn(move || {
            let mut planner = FftPlanner::new();
            let fft = planner.plan_fft_forward(window_size);

            // Create a complex vector
            let mut buffer: Vec<Complex<f32>> = fft_window
                .into_iter()
                .map(|x| Complex::new(x, 0.0))
                .collect();

            // Execute FFT
            fft.process(&mut buffer);

            let mut amp_spectrum: Vec<f32> = buffer.iter().map(|c| c.norm()).collect();

            for (i, value) in amp_spectrum.iter_mut().enumerate() {
                *value *= hanning_window[i];
            }

            let max_value = amp_spectrum
                .iter()
                .cloned()
                .max_by(|a, b| a.partial_cmp(b).unwrap())
                .unwrap();

            // Send the FFT result
            fft_result_tx.send(amp_spectrum.into_iter().map(|x| x / max_value).collect());
        });
    }

    pub fn subscribe(&self) -> SimpleReceiver<Vec<f32>> {
        self.fft_result_tx.subscribe()
    }
}
