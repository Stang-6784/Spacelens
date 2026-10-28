//! Procedurally drawn app icon (a violet tile with a pie slice), shared by build.rs and the app.

/// RGBA pixels for a square icon of `size` pixels.
pub fn rgba(size: u32) -> Vec<u8> {
    const SS: u32 = 4; // supersampling for smooth edges
    let s = size as f32;
    let radius = s * 0.24;
    let (cx, cy, pie_r) = (s * 0.5, s * 0.5, s * 0.29);
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let mut acc = [0f32; 4];
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f32 + (sx as f32 + 0.5) / SS as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SS as f32;
                    // rounded square mask
                    let dx = (px - s / 2.0).abs() - (s / 2.0 - radius);
                    let dy = (py - s / 2.0).abs() - (s / 2.0 - radius);
                    let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() > radius;
                    if outside {
                        continue;
                    }
                    // diagonal violet → blue gradient
                    let t = ((px + py) / (2.0 * s)).clamp(0.0, 1.0);
                    let mut c = [
                        139.0 + (56.0 - 139.0) * t,
                        92.0 + (130.0 - 92.0) * t,
                        246.0 + (246.0 - 246.0) * t,
                    ];
                    let (vx, vy) = (px - cx, py - cy);
                    let d = (vx * vx + vy * vy).sqrt();
                    if d < pie_r {
                        // Highlighted slice: the top-right quarter.
                        let slice = vx >= 0.0 && vy <= 0.0;
                        let a = if slice { 1.0 } else { 0.42 };
                        for ch in &mut c {
                            *ch = *ch + (255.0 - *ch) * a;
                        }
                    }
                    acc[0] += c[0];
                    acc[1] += c[1];
                    acc[2] += c[2];
                    acc[3] += 255.0;
                }
            }
            let n = (SS * SS) as f32;
            let i = ((y * size + x) * 4) as usize;
            let alpha = acc[3] / n;
            if alpha > 0.0 {
                let cover = alpha / 255.0;
                out[i] = (acc[0] / n / cover).round() as u8;
                out[i + 1] = (acc[1] / n / cover).round() as u8;
                out[i + 2] = (acc[2] / n / cover).round() as u8;
                out[i + 3] = alpha.round() as u8;
            }
        }
    }
    out
}
