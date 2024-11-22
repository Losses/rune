pub fn build_hanning_window(window_size: usize) -> Vec<f32> {
  (0..window_size)
      .map(|n| {
          0.5 * (1.0 - (2.0 * std::f32::consts::PI * n as f32 / (window_size as f32 - 1.0)).cos())
      })
      .collect()
}