// MS Remote ikonkasi (Android ikonkasi bilan bir xil chizma): ko'k yumaloq kvadrat, oq monitor, ichida "MS",
// burchakda kursor. Tashqi bog'liqliksiz: `build.rs` ham `include!` bilan ishlatadi (exe ikonkasi uchun .ico),
// dastur esa oyna va tray ikonkasi uchun.

/// `n x n` RGBA (to'g'ri alfa) rasm.
#[allow(dead_code)]
pub fn icon_rgba(n: u32) -> Vec<u8> {
    // Android vektoridagi (108 birlik) koordinatalar markazga nisbatan kattalashtiriladi
    const K: f32 = 1.4;
    let t = |x: f32, y: f32| ((x - 54.0) * K + 50.0, (y - 55.0) * K + 50.0);
    let line_w = 3.5 * K;
    let outline_w = 1.8 * K;

    // Oq chiziqlar: stend, "M" va "S"
    let mut segs: Vec<((f32, f32), (f32, f32))> = Vec::new();
    let mut poly = |pts: &[(f32, f32)]| {
        for w in pts.windows(2) {
            segs.push((t(w[0].0, w[0].1), t(w[1].0, w[1].1)));
        }
    };
    poly(&[(54.0, 70.0), (54.0, 77.0)]);
    poly(&[(45.0, 78.0), (63.0, 78.0)]);
    poly(&[(32.0, 59.0), (32.0, 43.0), (39.5, 53.0), (47.0, 43.0), (47.0, 59.0)]);
    let curves = [
        [(64.0, 46.0), (64.0, 42.0), (53.0, 41.5), (53.0, 46.5)],
        [(53.0, 46.5), (53.0, 51.5), (64.0, 50.5), (64.0, 55.5)],
        [(64.0, 55.5), (64.0, 60.5), (53.0, 59.5), (53.0, 55.5)],
    ];
    for c in curves {
        let mut pts = Vec::new();
        for i in 0..=16 {
            let u = i as f32 / 16.0;
            let v = 1.0 - u;
            let b = |a: f32, b: f32, c: f32, d: f32| v * v * v * a + 3.0 * v * v * u * b + 3.0 * v * u * u * c + u * u * u * d;
            pts.push((b(c[0].0, c[1].0, c[2].0, c[3].0), b(c[0].1, c[1].1, c[2].1, c[3].1)));
        }
        poly(&pts);
    }
    let cursor: Vec<(f32, f32)> =
        [(69.0, 55.0), (69.0, 73.0), (73.5, 69.0), (76.5, 76.0), (80.0, 74.5), (77.0, 67.5), (83.0, 67.0)].iter().map(|p| t(p.0, p.1)).collect();
    // Monitor: yumaloq to'rtburchak chegarasi (markaz, yarim o'lcham, burchak radiusi)
    let (m0, m1) = (t(27.0, 33.0), t(81.0, 70.0));
    let mon_c = ((m0.0 + m1.0) / 2.0, (m0.1 + m1.1) / 2.0);
    let mon_h = ((m1.0 - m0.0) / 2.0, (m1.1 - m0.1) / 2.0);
    let mon_r = 3.0 * K;

    const BLUE: [f32; 3] = [47.0, 107.0, 255.0];
    const WHITE: [f32; 3] = [255.0, 255.0, 255.0];
    const SS: u32 = 4;
    let scale = 100.0 / n as f32;
    let mut out = Vec::with_capacity((n * n * 4) as usize);
    for py in 0..n {
        for px in 0..n {
            let (mut r, mut g, mut b, mut a) = (0.0, 0.0, 0.0, 0.0);
            for sy in 0..SS {
                for sx in 0..SS {
                    let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) * scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) * scale;
                    let mut col = None;
                    if rbox_sdf((x, y), (50.0, 50.0), (48.0, 48.0), 22.0) <= 0.0 {
                        col = Some(BLUE);
                    }
                    if col.is_some() {
                        let on_line = segs.iter().any(|&(p, q)| seg_dist((x, y), p, q) <= line_w / 2.0);
                        let on_mon = rbox_sdf((x, y), mon_c, mon_h, mon_r).abs() <= line_w / 2.0;
                        if on_line || on_mon {
                            col = Some(WHITE);
                        }
                        if in_poly((x, y), &cursor) || poly_dist((x, y), &cursor) <= outline_w / 2.0 {
                            col = Some(if poly_dist((x, y), &cursor) <= outline_w / 2.0 { BLUE } else { WHITE });
                        }
                    }
                    if let Some(c) = col {
                        r += c[0];
                        g += c[1];
                        b += c[2];
                        a += 1.0;
                    }
                }
            }
            if a > 0.0 {
                out.extend_from_slice(&[(r / a) as u8, (g / a) as u8, (b / a) as u8, (a / (SS * SS) as f32 * 255.0).round() as u8]);
            } else {
                out.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    out
}

/// Yumaloq to'rtburchakning ishorali masofa funksiyasi (ichkarida manfiy).
fn rbox_sdf(p: (f32, f32), c: (f32, f32), half: (f32, f32), r: f32) -> f32 {
    let qx = (p.0 - c.0).abs() - (half.0 - r);
    let qy = (p.1 - c.1).abs() - (half.1 - r);
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

fn seg_dist(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let (cx, cy) = (a.0 + t * dx - p.0, a.1 + t * dy - p.1);
    (cx * cx + cy * cy).sqrt()
}

fn poly_dist(p: (f32, f32), pts: &[(f32, f32)]) -> f32 {
    (0..pts.len()).map(|i| seg_dist(p, pts[i], pts[(i + 1) % pts.len()])).fold(f32::MAX, f32::min)
}

fn in_poly(p: (f32, f32), pts: &[(f32, f32)]) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[j]);
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Windows .ico fayli (bir nechta o'lcham, 32-bit BMP yozuvlar).
#[allow(dead_code)]
pub fn icon_ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes
        .iter()
        .map(|&n| {
            let rgba = icon_rgba(n);
            let mask_row = ((n + 31) / 32 * 4) as usize;
            let mut d = Vec::new();
            // BITMAPINFOHEADER (balandlik ikki baravar: rang + AND niqob)
            for v in [40u32, n, n * 2] {
                d.extend_from_slice(&v.to_le_bytes());
            }
            d.extend_from_slice(&1u16.to_le_bytes());
            d.extend_from_slice(&32u16.to_le_bytes());
            for _ in 0..6 {
                d.extend_from_slice(&0u32.to_le_bytes());
            }
            for y in (0..n).rev() {
                for x in 0..n {
                    let i = ((y * n + x) * 4) as usize;
                    d.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
                }
            }
            d.extend(std::iter::repeat(0u8).take(mask_row * n as usize));
            d
        })
        .collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&n, img) in sizes.iter().zip(&images) {
        let b = if n >= 256 { 0 } else { n as u8 };
        out.extend_from_slice(&[b, b, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}
