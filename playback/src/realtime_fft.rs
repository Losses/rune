use rustfft::{num_complex::Complex, FftPlanner};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::broadcast;

pub struct RealTimeFFT {
    window_size: usize,
    window: Arc<Mutex<Vec<f32>>>,
    fft_result_tx: broadcast::Sender<Vec<f32>>,
}

impl RealTimeFFT {
    pub fn new(window_size: usize) -> Self {
        let (fft_result_tx, _) = broadcast::channel(16);
        let window = vec![0.0; window_size];
        RealTimeFFT {
            window_size,
            window: Arc::new(Mutex::new(window)),
            fft_result_tx,
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

            // Send the FFT result
            let _ = fft_result_tx.send(
                buffer
                    .into_iter()
                    .map(|x| ((x.re.powi(2) + x.im.powi(2)).sqrt()) / window_size as f32)
                    .collect(),
            );
        });
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Vec<f32>> {
        self.fft_result_tx.subscribe()
    }
}
