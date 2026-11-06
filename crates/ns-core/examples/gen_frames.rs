//! Write synthetic flickered PGM frames for the deflicker CLI.
//!
//! Banding comes from a 1 kHz square wave sampled at the row readout rate with a
//! 100 µs exposure window. Frame brightness pulses at 10 Hz.

use ns_core::pwm::synthetic_profile;
use std::fs;
use std::path::Path;

fn main() {
    let dir = std::env::args().nth(1).unwrap_or("frames".to_string());
    fs::create_dir_all(Path::new(&dir)).expect("mkdir");
    let line_rate = 40000.0;
    let rows = 256;
    let cols = 256;
    let mut phase = 0.0;
    for n in 0..60 {
        // 10 Hz strobe: half the frames catch the flash, half do not.
        let flash = (n % 10) < 3;
        let profile = synthetic_profile(100.0, 1000.0, phase, 0.3, line_rate, rows);
        phase += 0.013;
        let mut data = vec![0u8; cols * rows];
        for (r, v) in profile.iter().enumerate() {
            let base = if flash { 200.0 } else { 60.0 };
            let px = (base * v / 0.6).clamp(0.0, 255.0) as u8;
            for c in 0..cols {
                data[r * cols + c] = px;
            }
        }
        let name = format!("{dir}/frame_{:05}.pgm", n + 1);
        let header = format!("P5\n{cols} {rows}\n255\n");
        let mut bytes = header.into_bytes();
        bytes.extend(data);
        fs::write(name, bytes).expect("write");
    }
    println!("wrote 60 frames to {dir}");
}
