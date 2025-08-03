#[cfg(test)]
mod tests {
    use num_complex::Complex;
    use realfft::RealFftPlanner;
    use rustfft::FftPlanner;

    #[test]
    fn test_rust_fft() {
        let size = 1024;
        let original_data: Vec<Complex<f32>> = (0..size)
            .map(|i| Complex::new((i as f32).sin(), 0.0))
            .collect();
        let mut fft_data = original_data.clone();

        // Create FFT and IFFT planners
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(size);
        let ifft = planner.plan_fft_inverse(size);

        // Perform FFT
        fft.process(&mut fft_data);

        // Perform inverse FFT
        ifft.process(&mut fft_data);

        for x in fft_data.iter_mut() {
            *x /= size as f32;
        }

        // Compare original and reconstructed data
        for (orig, reconstructed) in original_data.iter().zip(fft_data.iter()) {
            let diff = (orig - reconstructed).norm();
            assert!(
                diff < 1e-6,
                "Difference too large: {orig} vs {reconstructed}, diff = {diff}"
            );
        }
    }

    #[test]
    fn test_real_fft() {
        let size = 1024;
        // Create real input data
        let mut real_input: Vec<f32> = (0..size).map(|i| (i as f32).sin()).collect();
        let mut real_output = real_input.clone();

        // Create real FFT planner
        let mut planner = RealFftPlanner::<f32>::new();
        let r2c = planner.plan_fft_forward(size);
        let c2r = planner.plan_fft_inverse(size);

        // Create complex buffer for FFT output
        let mut spectrum = r2c.make_output_vec();

        // Perform forward FFT
        r2c.process(&mut real_input, &mut spectrum).unwrap();

        // Perform inverse FFT
        c2r.process(&mut spectrum, &mut real_output).unwrap();

        // Scale the output
        for x in real_output.iter_mut() {
            *x /= size as f32;
        }

        // Compare original and reconstructed data
        for (orig, reconstructed) in real_input.iter().zip(real_output.iter()) {
            let diff = (orig - reconstructed).abs();
            assert!(
                diff < 1e-6,
                "Difference too large: {orig} vs {reconstructed}, diff = {diff}"
            );
        }
    }
}
