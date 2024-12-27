pub struct LowPassFilter {
    cutoff_freq: f64,
    sample_rate: f64,
    previous_output: f64,
}

impl LowPassFilter {
    pub fn new(cutoff_freq: f64, sample_rate: f64) -> Self {
        Self {
            cutoff_freq,
            sample_rate,
            previous_output: 0.0,
        }
    }

    fn filter(&mut self, sample: f64) -> f64 {
        let rc = 1.0 / (2.0 * std::f64::consts::PI * self.cutoff_freq);
        let dt = 1.0 / self.sample_rate;
        let alpha = dt / (rc + dt);
        self.previous_output = self.previous_output + alpha * (sample - self.previous_output);
        self.previous_output
    }

    pub fn filter_samples(&mut self, samples: &[f32]) -> Vec<f32> {
        samples
            .iter()
            .map(|&s| self.filter(s as f64) as f32)
            .collect()
    }
}
