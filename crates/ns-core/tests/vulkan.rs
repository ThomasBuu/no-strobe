#![cfg(feature = "vulkan")]

use ns_core::vulkan::VulkanCompute;

#[test]
fn test_vulkan_initialization() {
    match VulkanCompute::new() {
        Ok(_) => println!("VulkanCompute initialized successfully"),
        Err(e) => {
            println!("Vulkan not available on this environment: {e}");
        }
    }
}

#[test]
fn test_vulkan_row_reduction_accuracy() {
    let mut vk = match VulkanCompute::new() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Skipping test, Vulkan unavailable: {e}");
            return;
        }
    };

    let width = 640;
    let height = 480;
    let stride = width;
    let mut frame = vec![0u8; width * height];

    // Create synthetic pattern with distinct row averages
    for y in 0..height {
        let base = (y * 7 + 13) % 256;
        for x in 0..width {
            frame[y * stride + x] = ((base + x % 32) % 256) as u8;
        }
    }

    // CPU reference
    let cpu_means: Vec<f32> = frame
        .chunks_exact(stride)
        .map(|row| row.iter().map(|&v| v as f64).sum::<f64>() as f32 / (width as f32 * 255.0))
        .collect();

    // GPU execution
    let gpu_means = vk
        .compute_row_means(&frame, width, height, stride)
        .expect("GPU compute_row_means failed");

    assert_eq!(gpu_means.len(), height);
    for y in 0..height {
        let diff = (gpu_means[y] - cpu_means[y]).abs();
        assert!(
            diff < 1e-4,
            "Row {y} mean mismatch: GPU={}, CPU={}, diff={}",
            gpu_means[y],
            cpu_means[y],
            diff
        );
    }
}

#[test]
fn test_vulkan_deflicker_accuracy() {
    let mut vk = match VulkanCompute::new() {
        Ok(v) => v,
        Err(e) => {
            eprintln!("Skipping test, Vulkan unavailable: {e}");
            return;
        }
    };

    let width = 1920;
    let height = 1080;
    let stride = width;
    let mut src = vec![0u8; width * height];
    let mut dst = vec![0u8; width * height];

    for y in 0..height {
        for x in 0..width {
            src[y * stride + x] = ((x * 3 + y * 5) % 256) as u8;
        }
    }

    let mut row_gains = vec![1.0f32; height];
    for y in 0..height {
        row_gains[y] = 1.0 + 0.3 * ((y as f32 * 0.05).sin());
    }
    let temporal_gain = 1.15f32;

    // CPU reference
    let mut cpu_expected = vec![0u8; width * height];
    for y in 0..height {
        let g = (row_gains[y] * temporal_gain) as f64;
        for x in 0..width {
            let px = src[y * stride + x] as f64;
            let val = (px * g).round().clamp(0.0, 255.0) as u8;
            cpu_expected[y * stride + x] = val;
        }
    }

    // GPU execution
    vk.deflicker_frame(&src, &mut dst, width, height, stride, &row_gains, temporal_gain)
        .expect("GPU deflicker_frame failed");

    // Verify GPU output matches CPU within 1 LSB (fixed-point rounding difference tolerance)
    let mut max_diff = 0i32;
    for i in 0..src.len() {
        let diff = (dst[i] as i32 - cpu_expected[i] as i32).abs();
        if diff > max_diff {
            max_diff = diff;
        }
        assert!(
            diff <= 1,
            "Pixel {i} mismatch: GPU={}, CPU={}, diff={}",
            dst[i],
            cpu_expected[i],
            diff
        );
    }
    println!("Deflicker 1080p verified: max diff = {max_diff} LSB");
}


#[test]
fn test_vulkan_performance_benchmark() {
    let mut vk = match VulkanCompute::new() {
        Ok(v) => v,
        Err(_) => return,
    };

    let width = 1920;
    let height = 1080;
    let stride = width;
    let src = vec![128u8; width * height];
    let mut dst = vec![0u8; width * height];
    let row_gains = vec![1.05f32; height];

    // Warmup
    let _ = vk.deflicker_frame(&src, &mut dst, width, height, stride, &row_gains, 1.0);

    let iters = 50;
    let start = std::time::Instant::now();
    for _ in 0..iters {
        vk.deflicker_frame(&src, &mut dst, width, height, stride, &row_gains, 1.0)
            .expect("deflicker failed");
    }
    let elapsed = start.elapsed();
    let per_frame_ms = elapsed.as_secs_f64() * 1000.0 / iters as f64;
    println!("\n>>> Vulkan 1080p Deflicker: {:.3} ms per frame ({:.1} FPS) <<<\n", per_frame_ms, 1000.0 / per_frame_ms);

    let start_red = std::time::Instant::now();
    for _ in 0..iters {
        let _ = vk.compute_row_means(&src, width, height, stride)
            .expect("reduction failed");
    }
    let elapsed_red = start_red.elapsed();
    let per_frame_red_ms = elapsed_red.as_secs_f64() * 1000.0 / iters as f64;
    println!(">>> Vulkan 1080p Row Reduction: {:.3} ms per frame ({:.1} FPS) <<<\n", per_frame_red_ms, 1000.0 / per_frame_red_ms);
}
