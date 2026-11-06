//! no-strobe-post: fix a clip recorded by the app, or serve /fix for the phone.
//!
//!   no-strobe-post fix clip.mp4 [--line-rate HZ] [-o fixed.mp4]
//!   no-strobe-post serve [--listen 127.0.0.1:8765] [--token SECRET]
//!
//! Needs ffmpeg and ffprobe in PATH.

mod fix;
mod serve;

use std::path::PathBuf;

fn usage() -> ! {
    eprintln!(
        "usage:\n  no-strobe-post fix <clip.mp4> [--line-rate HZ] [-o fixed.mp4]\n  no-strobe-post serve [--listen ip:port] [--token SECRET]\n\n\
         --line-rate: sensor row readout rate in rows/s (0 = only flatten whole-frame pumping)\n\
         serve defaults to 127.0.0.1:8765; --token is mandatory off localhost."
    );
    std::process::exit(2);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let code = match args.next().as_deref() {
        Some("fix") => cmd_fix(args.collect()),
        Some("serve") => cmd_serve(args.collect()),
        _ => usage(),
    };
    if let Err(e) = code {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn cmd_fix(args: Vec<String>) -> Result<(), String> {
    let mut input: Option<PathBuf> = None;
    let mut output: Option<PathBuf> = None;
    let mut line_rate = 0.0;
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--line-rate" => line_rate = it.next().and_then(|v| v.parse().ok()).ok_or("--line-rate needs a number")?,
            "-o" | "--output" => output = Some(it.next().ok_or("-o needs a path")?.into()),
            _ if input.is_none() && !a.starts_with('-') => input = Some(a.into()),
            _ => usage(),
        }
    }
    let input = input.unwrap_or_else(|| usage());
    let output = output.unwrap_or_else(|| {
        let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "clip".into());
        input.with_file_name(format!("{stem}-fixed.mp4"))
    });
    let st = fix::fix(&input, &output, line_rate, &mut |m| eprintln!("{m}"))?;
    eprintln!(
        "{} frames, temporal gain {:.2}..{:.2}, banding {:.1} Hz -> {}",
        st.frames, st.gain_min, st.gain_max, st.banding_hz, output.display()
    );
    Ok(())
}

fn cmd_serve(args: Vec<String>) -> Result<(), String> {
    let mut listen = "127.0.0.1:8765".to_string();
    let mut token = std::env::var("NO_STROBE_TOKEN").ok().filter(|t| !t.is_empty());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--listen" => listen = it.next().ok_or("--listen needs ip:port")?,
            "--token" => token = Some(it.next().ok_or("--token needs a value")?),
            _ => usage(),
        }
    }
    serve::serve(&listen, token)
}
