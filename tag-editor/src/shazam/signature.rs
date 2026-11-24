use std::io::Cursor;

use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use crc32fast::hash;

use super::spectrogram::{FrequencyPeak, Signature};

const HEADER_SIZE: usize = 48;
const PEAKS_START: usize = HEADER_SIZE + 8;
const PEAK_RECORD_SIZE: usize = 5;

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
        _ => panic!("Invalid sample rate: {x}"),
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

            write_u32(&mut buf, 0x60030040 + band as u32);
            write_u32(&mut buf, peak_buf.len() as u32);
            buf.extend_from_slice(&peak_buf);
            buf.resize(buf.len().next_multiple_of(4), 0);
        }

        // Update lengths and checksum
        let content_len = (buf.len() - HEADER_SIZE) as u32;
        (&mut buf[8..12])
            .write_u32::<LittleEndian>(content_len)
            .unwrap();
        (&mut buf[52..56])
            .write_u32::<LittleEndian>(content_len)
            .unwrap();
        let checksum = hash(&buf[8..]);
        (&mut buf[4..8])
            .write_u32::<LittleEndian>(checksum)
            .unwrap();

        buf
    }

    pub fn decode(buf: &[u8]) -> Result<Self, String> {
        if buf.len() < PEAKS_START {
            return Err("buffer too short".into());
        }

        // Read and verify header
        let magic1 = (&buf[0..4])
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        if magic1 != 0xcafe2580 {
            return Err("bad magic1".into());
        }

        let checksum = (&buf[4..8])
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        let length = (&buf[8..12])
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;

        let calculated_checksum = hash(&buf[8..]);
        if checksum != calculated_checksum {
            return Err("bad checksum".into());
        }

        if length as usize != buf.len() - HEADER_SIZE {
            return Err("bad length".into());
        }

        let magic2 = (&buf[12..16])
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        if magic2 != 0x94119c00 {
            return Err("bad magic2".into());
        }

        let sample_rate_encoded = (&buf[28..32])
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        let sample_rate = convert_sample_rate((sample_rate_encoded >> 27) as i32);

        let samples_with_offset = (&buf[40..44])
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        let num_samples = (samples_with_offset as f64 - sample_rate as f64 * 0.24) as i32;

        let magic3 = (&buf[44..48])
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        if magic3 != 0x007c0000 {
            return Err("bad magic3".into());
        }

        let magic4 = (&buf[48..52])
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        if magic4 != 0x40000000 {
            return Err("bad magic4".into());
        }

        let length2 = (&buf[52..56])
            .read_u32::<LittleEndian>()
            .map_err(|e| e.to_string())?;
        // The first TLV repeats the message length after the 48-byte header.
        if length2 != length {
            return Err("bad length2".into());
        }

        let mut peaks_by_band = [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        let data = &buf[PEAKS_START..];
        let mut cursor = Cursor::new(data);

        // Read peaks
        while cursor.position() < data.len() as u64 {
            let band_info = cursor
                .read_u32::<LittleEndian>()
                .map_err(|e| e.to_string())?;
            let size = cursor
                .read_u32::<LittleEndian>()
                .map_err(|e| e.to_string())? as usize;
            let band = (band_info - 0x60030040) as usize;

            if band >= peaks_by_band.len() {
                return Err(format!("invalid band index: {band}"));
            }

            let start = cursor.position() as usize;
            let padding = (4 - size % 4) % 4;
            let block = size
                .checked_add(padding)
                .and_then(|len| data[start..].get(..len))
                .ok_or("invalid peak block length")?;
            cursor.set_position((start + block.len()) as u64);
            let mut peaks = &block[..size];

            // Older Rune encoders included zero padding in the declared peak length.
            let trailing = size % PEAK_RECORD_SIZE;
            if padding == 0
                && (1..=3).contains(&trailing)
                && peaks[size - trailing..].iter().all(|&byte| byte == 0)
            {
                peaks = &peaks[..size - trailing];
            }

            let mut records = Cursor::new(peaks);
            let mut pass = 0u32;
            while records.position() < peaks.len() as u64 {
                let offset = records.read_u8().map_err(|e| e.to_string())?;

                if offset == 0xFF {
                    pass = records
                        .read_u32::<LittleEndian>()
                        .map_err(|e| e.to_string())?;
                    continue;
                }

                pass += offset as u32;
                let magnitude = records
                    .read_u16::<LittleEndian>()
                    .map_err(|e| e.to_string())? as i32;
                let bin = records
                    .read_u16::<LittleEndian>()
                    .map_err(|e| e.to_string())? as i32;

                peaks_by_band[band].push(FrequencyPeak {
                    pass: pass as i32,
                    magnitude,
                    bin,
                });
            }
        }

        Ok(Self {
            sample_rate,
            num_samples,
            peaks_by_band,
        })
    }
}
