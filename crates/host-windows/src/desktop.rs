//! Joriy "input desktop"ga ulanish. Qulf ekrani, UAC va Ctrl+Alt+Del alohida desktop (`Winlogon`)da
//! ko'rsatiladi; oqim (thread) shu desktopga ulanmasa, `SendInput` va ekran olish faqat `Default`
//! desktopda ishlaydi. Winlogon desktopiga faqat SYSTEM huquqli jarayon ulana oladi; oddiy
//! foydalanuvchi jarayonida chaqiruv jimgina muvaffaqiyatsiz tugaydi va hech narsa o'zgarmaydi.

use std::cell::RefCell;
use windows_sys::Win32::{
    Foundation::FALSE,
    System::StationsAndDesktops::{
        CloseDesktop, GetThreadDesktop, GetUserObjectInformationW, OpenInputDesktop, SetThreadDesktop, HDESK, UOI_NAME,
    },
};

const GENERIC_ALL: u32 = 0x1000_0000;

struct Attached {
    handle: HDESK,
    name: String,
}

thread_local! {
    static CURRENT: RefCell<Option<Attached>> = const { RefCell::new(None) };
}

unsafe fn desktop_name(h: HDESK) -> String {
    let mut buf = [0u16; 128];
    let mut needed = 0u32;
    if GetUserObjectInformationW(h, UOI_NAME, buf.as_mut_ptr().cast(), (buf.len() * 2) as u32, &mut needed) == 0 {
        return String::new();
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

/// `attach_input_desktop` ning arzon varianti: har bir oqimda 200 ms dan kam tez-tez tekshirmaydi
/// (sichqoncha harakati paytida har hodisada `OpenInputDesktop` chaqirilmasligi uchun).
pub fn attach_throttled() -> bool {
    thread_local! {
        static LAST: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
    }
    let now = std::time::Instant::now();
    let due = LAST.with(|l| l.get().map_or(true, |t| now.duration_since(t) >= std::time::Duration::from_millis(200)));
    if !due {
        return false;
    }
    LAST.with(|l| l.set(Some(now)));
    attach_input_desktop()
}

/// Chaqiruvchi oqimni hozir foydalanuvchi kiritayotgan desktopga ulaydi (o'zgargan bo'lsagina).
/// `true` — desktop o'zgardi (ekran olish DC sini qayta yaratish kerak).
pub fn attach_input_desktop() -> bool {
    unsafe {
        let h = OpenInputDesktop(0, FALSE, GENERIC_ALL);
        if h.is_null() {
            return false; // ruxsat yo'q (SYSTEM emas) yoki desktop almashayotgan payt
        }
        let name = desktop_name(h);
        let same = CURRENT.with(|c| c.borrow().as_ref().map_or_else(|| desktop_name(GetThreadDesktop(
            windows_sys::Win32::System::Threading::GetCurrentThreadId(),
        )) == name, |a| a.name == name));
        if same {
            CloseDesktop(h);
            return false;
        }
        if SetThreadDesktop(h) == 0 {
            CloseDesktop(h);
            return false;
        }
        CURRENT.with(|c| {
            if let Some(old) = c.borrow_mut().replace(Attached { handle: h, name }) {
                CloseDesktop(old.handle);
            }
        });
        true
    }
}
