//! A small HTTP server the phone can offload clips to.
//!
//!   POST /fix?line_rate=<hz>   body = the mp4, reply = the fixed mp4
//!   GET  /health               "ok"
//!
//! One clip at a time (ffmpeg already uses every core). A token is required whenever the
//! server listens beyond the loopback interface: the request body is a video file and the
//! server runs ffmpeg on it, so it must not be open on a network.

use crate::fix::fix;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

const MAX_BODY: u64 = 4 << 30;
static COUNTER: AtomicU64 = AtomicU64::new(0);
static BUSY: Mutex<()> = Mutex::new(());

pub fn serve(listen: &str, token: Option<String>) -> Result<(), String> {
    let addr: SocketAddr = listen.parse().map_err(|_| format!("bad address {listen}, expected ip:port"))?;
    if !is_loopback(addr.ip()) && token.as_deref().map_or(true, str::is_empty) {
        return Err("listening beyond localhost requires --token (the server runs ffmpeg on uploaded files)".into());
    }
    let listener = TcpListener::bind(addr).map_err(|e| format!("cannot listen on {addr}: {e}"))?;
    eprintln!("no-strobe-post listening on {addr}{}", if token.is_some() { " (token required)" } else { "" });
    for conn in listener.incoming() {
        let Ok(stream) = conn else { continue };
        let token = token.clone();
        std::thread::spawn(move || {
            if let Err(e) = handle(stream, token.as_deref()) {
                eprintln!("request failed: {e}");
            }
        });
    }
    Ok(())
}

fn is_loopback(ip: IpAddr) -> bool {
    ip.is_loopback()
}

fn reply(s: &mut TcpStream, code: u16, text: &str, body: &str) {
    let _ = write!(
        s,
        "HTTP/1.1 {code} {text}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn handle(mut stream: TcpStream, token: Option<&str>) -> Result<(), String> {
    let mut reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    let mut parts = line.split_whitespace();
    let (method, target) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let mut length: Option<u64> = None;
    let mut auth = String::new();
    let mut expect_continue = false;
    let mut header_bytes = 0;
    loop {
        let mut h = String::new();
        let n = reader.read_line(&mut h).map_err(|e| e.to_string())?;
        header_bytes += n;
        if n == 0 || h == "\r\n" || h == "\n" {
            break;
        }
        if header_bytes > 16 * 1024 {
            reply(&mut stream, 431, "Request Header Fields Too Large", "headers too large");
            return Ok(());
        }
        if let Some((k, v)) = h.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            match k.as_str() {
                "content-length" => length = v.parse().ok(),
                "authorization" => auth = v.to_string(),
                "expect" => expect_continue = v.eq_ignore_ascii_case("100-continue"),
                _ => {}
            }
        }
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));

    if method == "GET" && path == "/health" {
        reply(&mut stream, 200, "OK", "ok");
        return Ok(());
    }
    if method != "POST" || path != "/fix" {
        reply(&mut stream, 404, "Not Found", "POST /fix or GET /health");
        return Ok(());
    }
    if let Some(t) = token {
        if auth.strip_prefix("Bearer ").map(str::trim) != Some(t) {
            reply(&mut stream, 401, "Unauthorized", "missing or wrong token");
            return Ok(());
        }
    }
    let Some(length) = length else {
        reply(&mut stream, 411, "Length Required", "Content-Length is required");
        return Ok(());
    };
    if length == 0 || length > MAX_BODY {
        reply(&mut stream, 413, "Payload Too Large", "empty or over 4 GiB");
        return Ok(());
    }
    let line_rate: f64 = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("line_rate="))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);

    let dir: PathBuf = std::env::temp_dir().join(format!(
        "no-strobe-post-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::SeqCst)
    ));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        if expect_continue {
            stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").map_err(|e| e.to_string())?;
        }
        let input = dir.join("in.mp4");
        let mut f = File::create(&input).map_err(|e| e.to_string())?;
        std::io::copy(&mut (&mut reader).take(length), &mut f).map_err(|e| format!("upload: {e}"))?;
        drop(f);
        let output = dir.join("out.mp4");
        let _one_at_a_time = BUSY.lock().unwrap_or_else(|e| e.into_inner());
        eprintln!("fixing {} bytes (line rate {line_rate})", length);
        let stats = fix(&input, &output, line_rate, &mut |m| eprintln!("  {m}"));
        match stats {
            Ok(st) => {
                let size = std::fs::metadata(&output).map_err(|e| e.to_string())?.len();
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {size}\r\nX-Frames: {}\r\nX-Gain-Range: {:.2}-{:.2}\r\nConnection: close\r\n\r\n",
                    st.frames, st.gain_min, st.gain_max
                )
                .map_err(|e| e.to_string())?;
                let mut out = File::open(&output).map_err(|e| e.to_string())?;
                let mut buf = vec![0u8; 1 << 20];
                loop {
                    let n = out.read(&mut buf).map_err(|e| e.to_string())?;
                    if n == 0 {
                        break;
                    }
                    stream.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                }
                Ok(())
            }
            Err(e) => {
                reply(&mut stream, 500, "Internal Server Error", &e);
                Ok(())
            }
        }
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}
