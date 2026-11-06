//! Offline de-flicker on ffmpeg-extracted frames.
//!
//!     ffmpeg -i clip.mp4 frame_%05d.pgm
//!     cargo run -p ns-core --example deflicker frames/ out/ <line_rate_hz>
//!
//! Frames are PGM (binary P5). Each frame gets a temporal gain (whole-frame
//! brightness) and a per-row gain (banding). Re-encode afterwards:
//!
//!     ffmpeg -framerate 30 -i out/frame_%05d.pgm -c:v libx264 fixed.mp4

use ns_core::restorer::{row_gain, temporal_gain};
use std::path::PathBuf;

struct Pgm {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

/// Parse a binary P5 PGM. Header tokens are whitespace separated, comments start with '#'.
fn read_pgm(path: &PathBuf) -> Pgm {
    let bytes = std::fs::read(path).expect("read frame");
    let mut i = 0;
    fn token(b: &[u8], i: &mut usize) -> String {
        while *i < b.len() && (b[*i].is_ascii_whitespace() || b[*i] == b'#') {
            if b[*i] == b'#' {
                while *i < b.len() && b[*i] != b'\n' {
                    *i += 1;
                }
            } else {
                *i += 1;
            }
        }
        let start = *i;
        while *i < b.len() && !b[*i].is_ascii_whitespace() {
            *i += 1;
        }
        String::from_utf8_lossy(&b[start..*i]).into_owned()
    }
    assert_eq!(token(&bytes, &mut i), "P5", "not a P5 PGM: {}", path.display());
    let width = token(&bytes, &mut i).parse().expect("width");
    let height = token(&bytes, &mut i).parse().expect("height");
    let max = token(&bytes, &mut i).parse::<usize>().expect("maxval");
    assert_eq!(max, 255, "only 8-bit PGM is supported");
    // One whitespace byte separates the header from the raster.
    let mut data = bytes[i + 1..].to_vec();
    data.resize(width * height, 0);
    Pgm { width, height, data }
}

fn write_pgm(path: &PathBuf, pgm: &Pgm) {
    let header = format!("P5\n{} {}\n255\n", pgm.width, pgm.height);
    std::fs::write(path, [header.as_bytes(), &pgm.data].concat()).expect("write frame");
}

/// Mean of a frame, normalized 0..1.
fn mean(data: &[u8]) -> f64 {
    data.iter().map(|v| *v as f64).sum::<f64>() / (data.len() as f64 * 255.0)
}

/// Mean luminance per row.
fn row_profile(pgm: &Pgm) -> Vec<f64> {
    (0..pgm.height)
        .map(|r| {
            let row = &pgm.data[r * pgm.width..(r + 1) * pgm.width];
            row.iter().map(|v| *v as f64).sum::<f64>() / (row.len() as f64 * 255.0)
        })
        .collect()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: deflicker <frames_dir> <out_dir> [line_rate_hz]");
        std::process::exit(1);
    }
    let dir = PathBuf::from(&args[1]);
    let out = PathBuf::from(&args[2]);
    let line_rate: f64 = args.get(3).map(|v| v.parse().unwrap()).unwrap_or(0.0);
    std::fs::create_dir_all(&out).expect("create out dir");

    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("read frames dir")
        .filter_map(|e| e.map(|e| e.path()).ok())
        .filter(|p| p.extension().map(|x| x == "pgm").unwrap_or(false))
        .collect();
    files.sort();
    if files.is_empty() {
        eprintln!("no .pgm frames in {}", dir.display());
        std::process::exit(1);
    }

    let frames: Vec<Pgm> = files.iter().map(|p| read_pgm(p)).collect();
    let means: Vec<f64> = frames.iter().map(|f| mean(&f.data)).collect();
    let t_gain = temporal_gain(&means, 15, 4.0);

    let mut f_hz = 0.0;
    for (idx, frame) in frames.iter().enumerate() {
        let mut data = frame.data.clone();
        if line_rate > 0.0 {
            let (g, f) = row_gain(&row_profile(frame), line_rate, 3.0);
            f_hz = f;
            for (r, gain) in g.iter().enumerate() {
                for c in 0..frame.width {
                    let v = data[r * frame.width + c] as f64 * gain;
                    data[r * frame.width + c] = v.round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        let g = t_gain[idx];
        for v in data.iter_mut() {
            *v = (*v as f64 * g).round().clamp(0.0, 255.0) as u8;
        }
        write_pgm(&out.join(files[idx].file_name().unwrap()), &Pgm { width: frame.width, height: frame.height, data });
    }
    println!(
        "{} frames, temporal gain {}..{}, banding f {:.1} Hz",
        frames.len(),
        t_gain.iter().copied().fold(f64::INFINITY, f64::min),
        t_gain.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        f_hz
    );
}
