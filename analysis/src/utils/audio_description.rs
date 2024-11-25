use rustfft::num_complex::Complex;
pub struct AudioDescription {
    pub sample_rate: u32,
    pub duration: f64,
    pub total_samples: usize,
    pub spectrum: Vec<Complex<f32>>,
    pub rms: f32,
    pub zcr: usize,
    pub energy: f32,
}

impl std::fmt::Debug for AudioDescription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioDescription")
            .field("sample_rate", &self.sample_rate)
            .field("duration", &self.duration)
            .field("total_samples", &self.total_samples)
            .field("spectrum_len", &self.spectrum.len())
            .field("rms", &self.rms)
            .field("zcr", &self.zcr)
            .field("energy", &self.energy)
            .finish()
    }
}
