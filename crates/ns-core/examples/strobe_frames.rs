//! Strobe path against a directory of PGM frames: checks the detected period
//! against a known flicker rate.
//!
//! ```sh
//! python3 tools/make_flicker_video.py 10 4 flick.mp4
//! ffmpeg -i flick.mp4 frames/frame_%05d.pgm
//! cargo run -p ns-core --example strobe_frames frames/ 60
//! ```

use ns_core::camera::CameraLimits;
use ns_core::controller::Controller;
use ns_core::controller::State;
use std::fs;

fn mean(data: &[u8]) -> f64 {
    data.iter().map(|v| *v as f64).sum::<f64>() / data.len() as f64 / 255.0
}

fn main() {
    let dir = std::env::args().nth(1).expect("frames dir");
    let fps: f64 = std::env::args()
        .nth(2)
        .map(|s| s.parse().unwrap())
        .unwrap_or(60.0);
    let mut files: Vec<_> = fs::read_dir(&dir)
        .expect("read dir")
        .filter_map(|e| e.map(|f| f.path()).ok())
        .filter(|p| p.extension().map(|x| x == "pgm").unwrap_or(false))
        .collect();
    files.sort();

    let mut c = Controller::new(CameraLimits::typical_phone());
    let frame_us = 1e6 / fps;
    for (i, path) in files.iter().enumerate() {
        let bytes = fs::read(path).expect("read frame");
        let mut pos = 2;
        let mut toks = Vec::new();
        while toks.len() < 3 {
            while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
                pos += 1;
            }
            let mut end = pos;
            while end < bytes.len() && !bytes[end].is_ascii_whitespace() {
                end += 1;
            }
            toks.push(String::from_utf8_lossy(&bytes[pos..end]).parse::<usize>().expect("pgm header"));
            pos = end;
        }
        let data = &bytes[pos + 1..];
        let cmd = c.on_frame(i as f64 * frame_us, mean(data));
        if i % 10 == 0 || cmd.state == State::Locked {
            println!(
                "frame {i:4}  {:?}  exposure {:>9.1}  T {:>9.1} Hz {:>7.1}  conf {:.2}",
                cmd.state,
                cmd.exposure_us,
                cmd.period_us,
                if cmd.period_us > 0.0 { 1e6 / cmd.period_us } else { 0.0 },
                cmd.confidence
            );
        }
    }
}
