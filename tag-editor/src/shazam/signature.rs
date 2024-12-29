use std::io::Cursor;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use crc32fast::hash;

use super::spectrogram::SpectralPeaks;

#[derive(Debug, Clone)]
pub struct FrequencyPeak {
    pub pass: i32,
    pub magnitude: i32,
    pub bin: i32,
}

#[derive(Debug)]
pub struct Signature {
    sample_rate: i32,
    num_samples: i32,
    peaks_by_band: [Vec<FrequencyPeak>; 5],
}

fn convert_sample_rate(x: i32) -> i32 {
    match x {
        1 => 8000,
        2 => 11025,
        3 => 16000,
        4 => 32000,
        5 => 44100,
        8000 => 1,
        11025 => 2,
        16000 => 3,
        32000 => 4,
        44100 => 5,
        _ => panic!("Invalid sample rate: {}", x),
    }
}

impl Signature {
    pub fn new(sample_rate: i32, num_samples: i32, peaks_by_band: [Vec<FrequencyPeak>; 5]) -> Self {
        Self {
            sample_rate,
            num_samples,
            peaks_by_band,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::new();

        // Helper function to write u32
        let write_u32 = |buf: &mut Vec<u8>, value: u32| {
            buf.write_u32::<LittleEndian>(value).unwrap();
        };

        // Write header
        write_u32(&mut buf, 0xcafe2580);
        write_u32(&mut buf, 0); // checksum placeholder
        write_u32(&mut buf, 0); // length placeholder
        write_u32(&mut buf, 0x94119c00);
        write_u32(&mut buf, 0);
        write_u32(&mut buf, 0);
        write_u32(&mut buf, 0);
        write_u32(
            &mut buf,
            (convert_sample_rate(self.sample_rate) as u32) << 27,
        );
        write_u32(&mut buf, 0);
        write_u32(&mut buf, 0);
        write_u32(
            &mut buf,
            self.num_samples as u32 + (self.sample_rate as f64 * 0.24) as u32,
        );
        write_u32(&mut buf, 0x007c0000);
        write_u32(&mut buf, 0x40000000);
        write_u32(&mut buf, 0); // length2 placeholder

        // Write peaks
        for (band, peaks) in self.peaks_by_band.iter().enumerate() {
            if peaks.is_empty() {
                continue;
            }

            let mut peak_buf = Vec::new();
            let mut pass = 0;

            for peak in peaks {
                if peak.pass - pass >= 255 {
                    peak_buf.push(0xFF);
                    peak_buf
                        .write_u32::<LittleEndian>(peak.pass as u32)
                        .unwrap();
                    pass = peak.pass;
                }
                peak_buf.push((peak.pass - pass) as u8);
                peak_buf
                    .write_u16::<LittleEndian>(peak.magnitude as u16)
                    .unwrap();
                peak_buf.write_u16::<LittleEndian>(peak.bin as u16).unwrap();
                pass = peak.pass;
            }

            // Pad to multiple of 4
            while peak_buf.len() % 4 != 0 {
                peak_buf.push(0x00);
            }

            write_u32(&mut buf, 0x60030040 + band as u32);
            write_u32(&mut buf, peak_buf.len() as u32);
            buf.extend_from_slice(&peak_buf);
        }

        // Update lengths and checksum
        let content_len = (buf.len() - 48) as u32;
        let checksum = hash(&buf[8..]);
        let mut cursor = Cursor::new(&mut buf);
        cursor.set_position(8);
        cursor.write_u32::<LittleEndian>(content_len).unwrap();
        cursor.set_position(52);
        cursor.write_u32::<LittleEndian>(content_len).unwrap();
        cursor.set_position(4);
        cursor.write_u32::<LittleEndian>(checksum).unwrap();

        buf
    }

    pub fn decode(buf: &[u8]) -> Result<Self, String> {
        let mut cursor = Cursor::new(buf);

        // Read and verify header
        let magic1 = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        if magic1 != 0xcafe2580 {
            return Err("bad magic1".into());
        }

        let checksum = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        let length = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;

        let calculated_checksum = hash(&buf[8..]);
        if checksum != calculated_checksum {
            return Err("bad checksum".into());
        }

        if length != (buf.len() - 48) as u32 {
            return Err("bad length".into());
        }

        let magic2 = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        if magic2 != 0x94119c00 {
            return Err("bad magic2".into());
        }

        // Skip 3 u32s
        cursor.set_position(cursor.position() + 12);

        let sample_rate_encoded = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        let sample_rate = convert_sample_rate((sample_rate_encoded >> 27) as i32);

        // Skip 2 u32s
        cursor.set_position(cursor.position() + 8);

        let samples_with_offset = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        let num_samples = (samples_with_offset as f64 - sample_rate as f64 * 0.24) as i32;

        let magic3 = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        if magic3 != 0x007c0000 {
            return Err("bad magic3".into());
        }

        let magic4 = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        if magic4 != 0x40000000 {
            return Err("bad magic4".into());
        }

        let length2 = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        if length2 != (buf.len() as u32 - 40) {
            return Err("bad length2".into());
        }

        let mut peaks_by_band = [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];

        // Read peaks
        while cursor.position() < buf.len() as u64 {
            let band_info = cursor
                .read_u32::<LittleEndian>()
                .map_err(|e| e.to_string())?;
            let size = cursor
                .read_u32::<LittleEndian>()
                .map_err(|e| e.to_string())?;
            let band = (band_info - 0x60030040) as usize;

            let mut pass = 0u32;
            let end_pos = cursor.position() + size as u64;

            while cursor.position() < end_pos {
                let offset = cursor.read_u8().map_err(|e| e.to_string())?;

                if offset == 0xFF {
                    pass = cursor
                        .read_u32::<LittleEndian>()
                        .map_err(|e| e.to_string())?;
                    continue;
                }

                pass += offset as u32;
                let magnitude = cursor
                    .read_u16::<LittleEndian>()
                    .map_err(|e| e.to_string())? as i32;
                let bin = cursor
                    .read_u16::<LittleEndian>()
                    .map_err(|e| e.to_string())? as i32;

                peaks_by_band[band].push(FrequencyPeak {
                    pass: pass as i32,
                    magnitude,
                    bin,
                });
            }

            // Align to 4-byte boundary
            let remainder = size % 4;
            if remainder != 0 {
                cursor.set_position(cursor.position() + (4 - remainder) as u64);
            }
        }

        Ok(Self {
            sample_rate,
            num_samples,
            peaks_by_band,
        })
    }
}

impl From<SpectralPeaks> for Signature {
    fn from(peaks: SpectralPeaks) -> Self {
        Signature::new(
            peaks.sample_rate,
            peaks.num_samples,
            peaks.peaks_by_band,
        )
    }
}