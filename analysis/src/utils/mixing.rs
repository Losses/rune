use symphonia::core::{
    audio::{AudioBuffer, Signal},
    conv::IntoSample,
    sample::Sample,
};

/// Append mono samples without re-reading channel descriptors for each frame.
/// Keep the same conversion, channel summation order and divisor as the original.
pub(crate) fn append_mixed<T: Sample + IntoSample<f32>>(buf: &AudioBuffer<T>, out: &mut Vec<f32>) {
    let frames = buf.frames();
    match buf.spec().channels.count() {
        1 => out.extend(
            buf.chan(0)[..frames]
                .iter()
                .map(|&v| -0.0 + IntoSample::<f32>::into_sample(v)),
        ),
        2 => out.extend(
            buf.chan(0)[..frames]
                .iter()
                .zip(&buf.chan(1)[..frames])
                .map(|(&left, &right)| {
                    ((-0.0 + IntoSample::<f32>::into_sample(left))
                        + IntoSample::<f32>::into_sample(right))
                        / 2.0
                }),
        ),
        channels => {
            out.reserve(frames);
            for i in 0..frames {
                out.push(
                    (0..channels)
                        .map(|ch| IntoSample::<f32>::into_sample(buf.chan(ch)[i]))
                        .sum::<f32>()
                        / channels as f32,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use symphonia::core::audio::{Channels, SignalSpec};
    fn check<T: Sample + IntoSample<f32> + Copy>(values: &[T]) {
        for channels in [1, 2, 6] {
            for frames in [1, 3, 15, 16, 17, 127, 4096] {
                let spec =
                    SignalSpec::new(44100, Channels::from_bits_truncate((1 << channels) - 1));
                let mut buf = AudioBuffer::<T>::new(frames as u64, spec);
                buf.render_reserved(Some(frames));
                for ch in 0..channels {
                    for (i, v) in buf.chan_mut(ch).iter_mut().enumerate() {
                        *v = values[(i + ch) % values.len()];
                    }
                }
                let mut expected = vec![0.125];
                for i in 0..frames {
                    expected.push(
                        (0..channels)
                            .map(|ch| IntoSample::<f32>::into_sample(buf.chan(ch)[i]))
                            .sum::<f32>()
                            / channels as f32,
                    );
                }
                let mut actual = vec![0.125];
                append_mixed(&buf, &mut actual);
                assert_eq!(
                    actual.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    expected.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                    "channels={channels} frames={frames}"
                );
            }
        }
    }
    #[test]
    fn mixing_preserves_integer_conversion_and_float_bits() {
        check(&[0i16, 1, -1, i16::MIN, i16::MAX, 12345]);
        check(&[0i32, 1, -1, i32::MIN, i32::MAX, 16777217, 16777219]);
        check(&[
            0.0f32,
            -0.0,
            1.0,
            -1.0,
            f32::from_bits(1),
            f32::MIN_POSITIVE,
            0.33333334,
            -0.125,
        ]);
    }
}
