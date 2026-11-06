//! Blinks the case HDD LED at a chosen frequency, to film a real, known flicker.
//!
//!     cargo run --release -p ns-core --example led_blink -- <device> <hz> [duty] [seconds]
//!     e.g. led_blink /dev/sdc 500 0.5 60
//!
//! The case LED shows SATA activity. While a request is outstanding on the drive it is on, so
//! during the "on" part of each period the tool reads random 4 KiB blocks back to back
//! (O_DIRECT, QD1, read-only), then idles until the next period. Use an SSD: a hard disk's
//! 10 ms seeks cannot make a 100 Hz+ square wave. Edges are quantised by one read (~0.1 ms).

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom};
use std::os::unix::fs::{FileExt, OpenOptionsExt};
use std::time::{Duration, Instant};

const O_DIRECT: i32 = 0x4000;
const BLOCK: usize = 4096;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: led_blink <device> <hz> [duty] [seconds]");
        std::process::exit(2);
    }
    let dev = &a[1];
    let hz: f64 = a[2].parse().expect("hz");
    let duty: f64 = a.get(3).and_then(|v| v.parse().ok()).unwrap_or(0.5);
    let secs: f64 = a.get(4).and_then(|v| v.parse().ok()).unwrap_or(30.0);
    let period = Duration::from_secs_f64(1.0 / hz);

    let mut f = OpenOptions::new().read(true).custom_flags(O_DIRECT).open(dev).expect("open device (disk group?)");
    let size = f.seek(SeekFrom::End(0)).expect("device size");
    let blocks = (size / BLOCK as u64).max(1);

    let layout = Layout::from_size_align(BLOCK, BLOCK).unwrap();
    let ptr = unsafe { alloc_zeroed(layout) };
    let buf = unsafe { std::slice::from_raw_parts_mut(ptr, BLOCK) };

    let mut seed = 0x9E3779B97F4A7C15u64;
    let start = Instant::now();
    let end = start + Duration::from_secs_f64(secs);
    let mut cycles = 0u64;
    let mut reads = 0u64;
    let mut late = Duration::ZERO;
    let mut n = 0u64;
    while Instant::now() < end {
        let t0 = start + period.mul_f64(n as f64);
        let t_on_end = t0 + period.mul_f64(duty);
        let t_next = start + period.mul_f64((n + 1) as f64);
        // Wait for the cycle start (sleep most of it, spin the last 200 us).
        sleep_until(t0);
        while Instant::now() < t_on_end {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let off = (seed >> 20) % blocks * BLOCK as u64;
            f.read_exact_at(buf, off).expect("read");
            reads += 1;
        }
        let now = Instant::now();
        if now > t_next {
            late += now - t_next;
        }
        cycles += 1;
        n += 1;
    }
    unsafe { dealloc(ptr, layout) };
    let el = start.elapsed().as_secs_f64();
    println!(
        "{cycles} cycles in {el:.2} s = {:.2} Hz asked {hz} Hz, duty {duty}, {:.1} reads/cycle, overrun {:.2} ms total",
        cycles as f64 / el,
        reads as f64 / cycles.max(1) as f64,
        late.as_secs_f64() * 1e3
    );
}

fn sleep_until(t: Instant) {
    loop {
        let now = Instant::now();
        if now >= t {
            return;
        }
        let left = t - now;
        if left > Duration::from_micros(300) {
            std::thread::sleep(left - Duration::from_micros(200));
        } else {
            std::hint::spin_loop();
        }
    }
}
