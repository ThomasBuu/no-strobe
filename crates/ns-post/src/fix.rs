//! Fix one clip: de-flicker the luma with the `ns-core` restorer, keep the original
//! chroma, timing and rotation. Two streaming passes through ffmpeg raw pipes, so memory
//! stays at a few frames whatever the clip length.

use ns_core::restorer::{row_gain, temporal_gain};
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

pub struct Stats {
    pub frames: usize,
    pub gain_min: f64,
    pub gain_max: f64,
    pub banding_hz: f64,
}

struct Probe {
    width: usize,
    height: usize,
    /// Average frame rate as ffmpeg's "num/den" (clips from a phone are variable rate).
    fps: String,
    /// Counter-clockwise display rotation stored in the file, in degrees.
    rotation: i32,
}

fn run_capture(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("cannot run {:?}: {e}", cmd.get_program()))?;
    if !out.status.success() {
        return Err(format!("{:?} failed: {}", cmd.get_program(), String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn probe(input: &Path) -> Result<Probe, String> {
    let txt = run_capture(Command::new("ffprobe").args([
        "-v", "error", "-select_streams", "v:0",
        "-show_entries", "stream=width,height,avg_frame_rate:stream_side_data=rotation",
        "-of", "default=nw=1",
    ]).arg(input))?;
    let mut width = 0;
    let mut height = 0;
    let mut fps = String::from("30/1");
    let mut rotation = 0;
    for line in txt.lines() {
        if let Some((k, v)) = line.split_once('=') {
            match k {
                "width" => width = v.parse().map_err(|_| "bad width")?,
                "height" => height = v.parse().map_err(|_| "bad height")?,
                "avg_frame_rate" if v != "0/0" => fps = v.to_string(),
                "rotation" => rotation = v.parse::<f64>().map(|r| r.round() as i32).unwrap_or(0),
                _ => {}
            }
        }
    }
    if width == 0 || height == 0 {
        return Err("no video stream found".into());
    }
    Ok(Probe { width, height, fps, rotation })
}

fn decoder(input: &Path) -> Result<std::process::Child, String> {
    Command::new("ffmpeg")
        // passthrough: phone clips are variable frame rate, and ffmpeg would otherwise
        // duplicate frames to a constant rate (663 frames out of 88 on a real clip).
        .args(["-v", "error", "-noautorotate", "-i"])
        .arg(input)
        .args(["-an", "-fps_mode", "passthrough", "-f", "rawvideo", "-pix_fmt", "gray", "-"])
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))
}

fn mean(frame: &[u8]) -> f64 {
    frame.iter().map(|v| *v as f64).sum::<f64>() / (frame.len() as f64 * 255.0)
}

fn row_means(frame: &[u8], width: usize) -> Vec<f64> {
    frame
        .chunks(width)
        .map(|r| r.iter().map(|v| *v as f64).sum::<f64>() / (r.len() as f64 * 255.0))
        .collect()
}

/// `line_rate_hz`: sensor row readout rate; 0 only flattens whole-frame pumping.
pub fn fix(input: &Path, output: &Path, line_rate_hz: f64, log: &mut dyn FnMut(&str)) -> Result<Stats, String> {
    let p = probe(input)?;
    let frame_len = p.width * p.height;
    log(&format!("{}x{} at {} fps, rotation {}", p.width, p.height, p.fps, p.rotation));

    // Pass 1: brightness of every frame.
    let mut dec = decoder(input)?;
    let mut out = dec.stdout.take().ok_or("no decoder pipe")?;
    let mut buf = vec![0u8; frame_len];
    let mut means = Vec::new();
    while read_frame(&mut out, &mut buf)? {
        means.push(mean(&buf));
    }
    let _ = dec.wait();
    if means.is_empty() {
        return Err("no frames decoded".into());
    }
    let gains = temporal_gain(&means, 15, 4.0);
    log(&format!("pass 1: {} frames", means.len()));

    // Pass 2: apply the gains and encode, taking chroma, audio and rotation from the original.
    let mut enc = Command::new("ffmpeg")
        // -noautorotate on both inputs: the luma and the chroma stay in the stored orientation
        // and the rotation is written back as the display matrix of the output.
        .args(["-v", "error", "-y", "-noautorotate", "-f", "rawvideo", "-pix_fmt", "gray", "-s"])
        .arg(format!("{}x{}", p.width, p.height))
        .args(["-framerate", &p.fps])
        .args(["-display_rotation", &p.rotation.to_string(), "-i", "pipe:0"])
        .args(["-noautorotate", "-i"])
        .arg(input)
        .args([
            "-filter_complex",
            "[1:v]format=yuv444p,setsar=1,extractplanes=u+v[u][v];[0:v]format=gray,setsar=1[y];[y][u][v]mergeplanes=0x001020:yuv444p,format=yuv420p[o]",
            "-map", "[o]", "-map", "1:a?", "-c:v", "libx264", "-crf", "18", "-c:a", "copy", "-shortest",
        ])
        .arg(output)
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run ffmpeg: {e}"))?;
    let mut enc_in = enc.stdin.take().ok_or("no encoder pipe")?;

    let mut dec = decoder(input)?;
    let mut out = dec.stdout.take().ok_or("no decoder pipe")?;
    let mut idx = 0usize;
    let mut f_hz = 0.0;
    while idx < gains.len() && read_frame(&mut out, &mut buf)? {
        if line_rate_hz > 0.0 {
            let (g, f) = row_gain(&row_means(&buf, p.width), line_rate_hz, 3.0);
            if f > 0.0 {
                f_hz = f;
            }
            for (r, row) in buf.chunks_mut(p.width).enumerate() {
                let gr = g.get(r).copied().unwrap_or(1.0);
                for v in row.iter_mut() {
                    *v = (*v as f64 * gr).round().clamp(0.0, 255.0) as u8;
                }
            }
        }
        let g = gains[idx];
        for v in buf.iter_mut() {
            *v = (*v as f64 * g).round().clamp(0.0, 255.0) as u8;
        }
        enc_in.write_all(&buf).map_err(|e| format!("encoder closed early: {e}"))?;
        idx += 1;
    }
    drop(enc_in);
    // Drain the decoder so it ends cleanly instead of failing on a closed pipe.
    while read_frame(&mut out, &mut buf).unwrap_or(false) {}
    let _ = dec.wait();
    let st = enc.wait().map_err(|e| format!("ffmpeg: {e}"))?;
    if !st.success() {
        return Err("ffmpeg could not encode the result".into());
    }
    log(&format!("pass 2: {idx} frames written to {}", output.display()));
    Ok(Stats {
        frames: idx,
        gain_min: gains.iter().copied().fold(f64::INFINITY, f64::min),
        gain_max: gains.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        banding_hz: f_hz,
    })
}

/// Reads exactly one frame; false at a clean end of stream.
fn read_frame(r: &mut impl Read, buf: &mut [u8]) -> Result<bool, String> {
    let mut got = 0;
    while got < buf.len() {
        match r.read(&mut buf[got..]) {
            Ok(0) => return if got == 0 { Ok(false) } else { Err("truncated frame".into()) },
            Ok(n) => got += n,
            Err(e) => return Err(format!("read error: {e}")),
        }
    }
    Ok(true)
}
