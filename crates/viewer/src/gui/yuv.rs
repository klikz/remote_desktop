//! YUV420 (I420) -> RGBA (`egui::Color32`) o'girish: butun sonli arifmetika, qatorlar bo'yicha parallel.
//! openh264 ning `write_rgb8` (RGB, keyin UI oqimida yana RGBA ga) o'rniga to'g'ridan-to'g'ri tekstura formatiga yoziladi.
//! Koeffitsientlar openh264 dagi bilan bir xil (BT.601, to'liq diapazon), shuning uchun ranglar o'zgarmaydi.

use egui::Color32;

/// 16 bitli qo'zg'almas nuqta koeffitsientlari.
const RV: i32 = 91_881; // 1.402
const GU: i32 = 22_544; // 0.344
const GV: i32 = 46_793; // 0.714
const BU: i32 = 116_130; // 1.772

#[inline]
fn clamp(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// Bitta qatorni o'giradi.
#[inline]
fn row(y_row: &[u8], u_row: &[u8], v_row: &[u8], out: &mut [Color32]) {
    let w = out.len();
    for (i, px) in out.iter_mut().enumerate().take(w) {
        let c = i / 2;
        let y = i32::from(y_row[i]) << 16;
        let u = i32::from(u_row[c]) - 128;
        let v = i32::from(v_row[c]) - 128;
        let r = (y + RV * v) >> 16;
        let g = (y - GU * u - GV * v) >> 16;
        let b = (y + BU * u) >> 16;
        *px = Color32::from_rgb(clamp(r), clamp(g), clamp(b));
    }
}

/// `y` qatoriga mos xrom qatori. Toq balandlikda oxirgi qator bufer chegarasidan chiqsa, oldingisi ishlatiladi.
fn chroma(plane: &[u8], stride: usize, y: usize, cw: usize) -> &[u8] {
    let mut r = y / 2;
    while r > 0 && r * stride + cw > plane.len() {
        r -= 1;
    }
    &plane[r * stride..r * stride + cw]
}

/// `planes`: (Y, U, V), `strides`: (Y, U, V) qator qadamlari, `dim`: (kenglik, balandlik).
pub fn to_color32(planes: (&[u8], &[u8], &[u8]), strides: (usize, usize, usize), dim: (usize, usize)) -> Vec<Color32> {
    let (w, h) = dim;
    let mut out = vec![Color32::BLACK; w * h];
    if w == 0 || h == 0 {
        return out;
    }
    let cw = w.div_ceil(2);
    let convert = |y0: usize, chunk: &mut [Color32]| {
        for (k, line) in chunk.chunks_exact_mut(w).enumerate() {
            let y = y0 + k;
            let yr = &planes.0[y * strides.0..y * strides.0 + w];
            row(yr, chroma(planes.1, strides.1, y, cw), chroma(planes.2, strides.2, y, cw), line);
        }
    };
    // Katta kadrlarda bir nechta oqim (1080p da ~2 mln piksel)
    let threads = if w * h >= 1280 * 720 {
        std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 4)
    } else {
        1
    };
    if threads == 1 {
        convert(0, &mut out);
        return out;
    }
    let rows_per = h.div_ceil(threads);
    std::thread::scope(|s| {
        for (i, chunk) in out.chunks_mut(rows_per * w).enumerate() {
            let convert = &convert;
            s.spawn(move || convert(i * rows_per, chunk));
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grey_stays_grey_and_matches_size() {
        let (w, h) = (6, 4);
        let y = vec![128u8; w * h];
        let u = vec![128u8; 3 * 2];
        let v = vec![128u8; 3 * 2];
        let px = to_color32((&y, &u, &v), (w, 3, 3), (w, h));
        assert_eq!(px.len(), w * h);
        assert!(px.iter().all(|c| *c == Color32::from_rgb(128, 128, 128)));
    }

    #[test]
    fn odd_height_does_not_read_past_chroma() {
        // openh264 xrom tekisligini `balandlik * qadam / 2` uzunlikda beradi: toq balandlikda bir qator kam
        let (w, h) = (4, 3);
        let y = vec![100u8; w * h];
        let u = vec![128u8; h * 2 / 2];
        let v = vec![128u8; h * 2 / 2];
        let px = to_color32((&y, &u, &v), (w, 2, 2), (w, h));
        assert_eq!(px.len(), w * h);
    }

    #[test]
    fn red_like_openh264() {
        // BT.601 qizil: Y=76 U=85 V=255 -> openh264 formulasi bo'yicha (254, 0, 0) atrofida
        let px = to_color32((&[76, 76], &[85], &[255]), (2, 1, 1), (2, 1));
        let c = px[0];
        assert!(c.r() >= 250 && c.g() <= 3 && c.b() <= 3, "{c:?}");
    }

    #[test]
    fn parallel_matches_single_thread() {
        let (w, h) = (1280, 720);
        let y: Vec<u8> = (0..w * h).map(|i| (i % 251) as u8).collect();
        let u: Vec<u8> = (0..(w / 2) * (h / 2)).map(|i| (i % 241) as u8).collect();
        let v: Vec<u8> = (0..(w / 2) * (h / 2)).map(|i| (i % 239) as u8).collect();
        let par = to_color32((&y, &u, &v), (w, w / 2, w / 2), (w, h));
        let mut one = vec![Color32::BLACK; w * h];
        for (yy, line) in one.chunks_exact_mut(w).enumerate() {
            row(&y[yy * w..yy * w + w], &u[(yy / 2) * (w / 2)..], &v[(yy / 2) * (w / 2)..], line);
        }
        assert!(par == one);
    }
}
