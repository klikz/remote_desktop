//! Asosiy monitor ekranini GDI orqali olish (kursor bilan). Oddiy va ishonchli; keyinroq DXGI Desktop
//! Duplication bilan almashtirish mumkin (tezroq, GPU).

use anyhow::{bail, Result};
use std::{mem, ptr};
use windows_sys::Win32::{
    Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, HBITMAP, HDC, HGDIOBJ,
    },
    UI::WindowsAndMessaging::{
        DrawIconEx, GetCursorInfo, GetIconInfo, GetSystemMetrics, CURSORINFO, ICONINFO, SM_CXSCREEN, SM_CYSCREEN,
    },
};

const SRCCOPY: u32 = 0x00CC_0020;
const CAPTUREBLT: u32 = 0x4000_0000;
const CURSOR_SHOWING: u32 = 1;
const DI_NORMAL: u32 = 3;

pub struct Grabber {
    screen_dc: HDC,
    mem_dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    pub w: usize,
    pub h: usize,
}

impl Grabber {
    pub fn screen_size() -> (usize, usize) {
        unsafe { (GetSystemMetrics(SM_CXSCREEN).max(2) as usize, GetSystemMetrics(SM_CYSCREEN).max(2) as usize) }
    }

    pub fn new() -> Result<Self> {
        let (w, h) = Self::screen_size();
        unsafe {
            let screen_dc = GetDC(ptr::null_mut());
            if screen_dc.is_null() {
                bail!("ekran DC sini olib bo'lmadi");
            }
            let mem_dc = CreateCompatibleDC(screen_dc);
            if mem_dc.is_null() {
                ReleaseDC(ptr::null_mut(), screen_dc);
                bail!("xotira DC sini yaratib bo'lmadi");
            }
            let mut bmi: BITMAPINFO = mem::zeroed();
            bmi.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = w as i32;
            bmi.bmiHeader.biHeight = -(h as i32); // yuqoridan pastga
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = 0; // BI_RGB
            let mut bits: *mut core::ffi::c_void = ptr::null_mut();
            let bmp = CreateDIBSection(mem_dc, &bmi, 0, &mut bits, ptr::null_mut(), 0);
            if bmp.is_null() || bits.is_null() {
                DeleteDC(mem_dc);
                ReleaseDC(ptr::null_mut(), screen_dc);
                bail!("DIB bitmap yaratib bo'lmadi ({w}x{h})");
            }
            let old = SelectObject(mem_dc, bmp);
            Ok(Grabber { screen_dc, mem_dc, bmp, old, bits: bits as *mut u8, w, h })
        }
    }

    /// Ekran o'lchami o'zgarganmi (ruxsat o'zgardi, monitor almashdi)?
    pub fn size_changed(&self) -> bool {
        Self::screen_size() != (self.w, self.h)
    }

    /// Joriy ekranni (kursor bilan) oladi. Natija: BGRA (B, G, R, 0), yuqoridan pastga.
    pub fn grab(&mut self) -> &[u8] {
        unsafe {
            BitBlt(self.mem_dc, 0, 0, self.w as i32, self.h as i32, self.screen_dc, 0, 0, SRCCOPY | CAPTUREBLT);
            self.draw_cursor();
            std::slice::from_raw_parts(self.bits, self.w * self.h * 4)
        }
    }

    unsafe fn draw_cursor(&self) {
        let mut ci: CURSORINFO = mem::zeroed();
        ci.cbSize = mem::size_of::<CURSORINFO>() as u32;
        if GetCursorInfo(&mut ci) == 0 || ci.flags & CURSOR_SHOWING == 0 || ci.hCursor.is_null() {
            return;
        }
        let (mut hx, mut hy) = (0i32, 0i32);
        let mut ii: ICONINFO = mem::zeroed();
        if GetIconInfo(ci.hCursor, &mut ii) != 0 {
            hx = ii.xHotspot as i32;
            hy = ii.yHotspot as i32;
            if !ii.hbmMask.is_null() {
                DeleteObject(ii.hbmMask);
            }
            if !ii.hbmColor.is_null() {
                DeleteObject(ii.hbmColor);
            }
        }
        DrawIconEx(self.mem_dc, ci.ptScreenPos.x - hx, ci.ptScreenPos.y - hy, ci.hCursor, 0, 0, 0, ptr::null_mut(), DI_NORMAL);
    }
}

impl Drop for Grabber {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.mem_dc, self.old);
            DeleteObject(self.bmp);
            DeleteDC(self.mem_dc);
            ReleaseDC(ptr::null_mut(), self.screen_dc);
        }
    }
}

/// BGRA kadrni `tw x th` o'lchamga kichraytiradi (quti filtri) va alfa kanalini to'ldiradi.
pub fn scale_bgra(src: &[u8], sw: usize, sh: usize, tw: usize, th: usize, out: &mut Vec<u8>) {
    out.resize(tw * th * 4, 0);
    if tw == sw && th == sh {
        out.copy_from_slice(&src[..sw * sh * 4]);
        return;
    }
    // Katta kadrlarda (4K -> 1080p: ~8 mln manba piksel) qatorlar bir nechta oqimga bo'linadi: bu sikl har kadrda
    // ishlaydi va bitta yadroda kadr oralig'ining katta qismini egallaydi.
    let threads = if tw * th >= 1280 * 720 {
        std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 4)
    } else {
        1
    };
    if threads == 1 {
        scale_rows(src, sw, sh, tw, th, 0, out);
        return;
    }
    let rows_per = th.div_ceil(threads);
    std::thread::scope(|s| {
        for (i, chunk) in out.chunks_mut(rows_per * tw * 4).enumerate() {
            s.spawn(move || scale_rows(src, sw, sh, tw, th, i * rows_per, chunk));
        }
    });
}

/// `out`: `y0` qatoridan boshlanadigan to'liq qatorlar (har biri `tw * 4` bayt).
fn scale_rows(src: &[u8], sw: usize, sh: usize, tw: usize, th: usize, y0: usize, out: &mut [u8]) {
    for (k, line) in out.chunks_exact_mut(tw * 4).enumerate() {
        let y = y0 + k;
        let sy0 = y * sh / th;
        let sy1 = ((y + 1) * sh / th).clamp(sy0 + 1, sh);
        for (x, px) in line.chunks_exact_mut(4).enumerate() {
            let sx0 = x * sw / tw;
            let sx1 = ((x + 1) * sw / tw).clamp(sx0 + 1, sw);
            let (mut b, mut g, mut r) = (0u32, 0u32, 0u32);
            for sy in sy0..sy1 {
                for p in src[(sy * sw + sx0) * 4..(sy * sw + sx1) * 4].chunks_exact(4) {
                    b += u32::from(p[0]);
                    g += u32::from(p[1]);
                    r += u32::from(p[2]);
                }
            }
            let n = ((sy1 - sy0) * (sx1 - sx0)) as u32;
            px[0] = (b / n) as u8;
            px[1] = (g / n) as u8;
            px[2] = (r / n) as u8;
            px[3] = 255;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parallel yo'l (katta kadr) bitta oqimdagi bilan bir xil natija beradi.
    #[test]
    fn parallel_scale_matches_single_thread() {
        let (sw, sh, tw, th) = (2560usize, 1441usize, 1280usize, 720usize);
        let src: Vec<u8> = (0..sw * sh * 4).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        let mut par = Vec::new();
        scale_bgra(&src, sw, sh, tw, th, &mut par);
        let mut one = vec![0u8; tw * th * 4];
        scale_rows(&src, sw, sh, tw, th, 0, &mut one);
        assert_eq!(par, one);
    }

    #[test]
    fn same_size_is_copied() {
        let src = vec![7u8; 4 * 2 * 4];
        let mut out = Vec::new();
        scale_bgra(&src, 4, 2, 4, 2, &mut out);
        assert_eq!(out, src);
    }
}
