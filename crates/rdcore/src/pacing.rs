//! Video oqimini boshqarish: kalit kadr so'rovlari va kechikishni cheklash.
//!
//! Host har yuborilgan kadrni (`pts_us`) eslab qoladi, Viewer esa oxirgi qabul qilgan kadrini `Feedback` bilan
//! bildiradi. Ular orasidagi farq "yo'ldagi" kadrlar: tarmoq yoki Viewer ulgurmayotgan bo'lsa ular ko'payadi va Host
//! kodlashni vaqtincha to'xtatadi (kodlagichdan **oldin**, shuning uchun H.264 zanjiri uzilmaydi va IDR kerak bo'lmaydi).
//! Natijada kechikish QUIC buferlari hajmiga emas, shu yerdagi chegaraga bog'liq bo'ladi.

use protocol::InputEvent;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

/// Viewer `Feedback` ni shu oraliqda yuboradi (yangi kadr kelgan bo'lsa).
pub const FEEDBACK_INTERVAL: Duration = Duration::from_millis(50);
/// RTT va javob oralig'idan tashqari ruxsat etilgan navbat (kechikish chegarasi).
const TARGET_QUEUE: Duration = Duration::from_millis(150);
/// Yo'ldagi kadrlar sonining pastki chegarasi (juda past FPS/RTT da ham oqim to'xtab qolmasin).
const MIN_IN_FLIGHT: usize = 4;
/// Viewer dekoder navbatida shuncha kadr bo'lsa, u ulgurmayapti: Host kutadi.
const VIEWER_QUEUE_LIMIT: i32 = 4;
/// Viewer kalit kadrni shundan tez-tez so'ramaydi.
const KEYFRAME_REQUEST_GAP: Duration = Duration::from_millis(300);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Host tomoni: bitta video oqimi uchun umumiy holat (control oqimi, kodlagich va yuboruvchi o'rtasida).
pub struct VideoControl {
    fps: AtomicU32,
    want_key: AtomicBool,
    rtt_us: AtomicU64,
    viewer_queue: AtomicI32,
    /// Yuborilgan, lekin Viewer hali tasdiqlamagan kadrlar (`pts_us`, o'sish tartibida).
    sent: Mutex<VecDeque<u64>>,
}

impl VideoControl {
    pub fn new(fps: u32) -> Self {
        VideoControl {
            fps: AtomicU32::new(fps.max(1)),
            want_key: AtomicBool::new(false),
            rtt_us: AtomicU64::new(50_000),
            viewer_queue: AtomicI32::new(0),
            sent: Mutex::new(VecDeque::new()),
        }
    }

    /// Viewer kalit kadr so'radi.
    pub fn request_keyframe(&self) {
        self.want_key.store(true, Ordering::Relaxed);
    }

    /// Kalit kadr so'ralganmi (so'rov olinadi va tozalanadi).
    pub fn take_keyframe_request(&self) -> bool {
        self.want_key.swap(false, Ordering::Relaxed)
    }

    pub fn set_rtt(&self, rtt: Duration) {
        self.rtt_us.store(rtt.as_micros() as u64, Ordering::Relaxed);
    }

    /// Kadr Viewer'ga yuborildi.
    pub fn on_sent(&self, pts_us: u64) {
        let mut q = lock(&self.sent);
        q.push_back(pts_us);
        // Viewer umuman javob bermasa ham xotira o'smasin
        while q.len() > 1024 {
            q.pop_front();
        }
    }

    /// Viewer `Feedback` yubordi: `last_pts_us` gacha bo'lgan kadrlar yetib bordi.
    pub fn on_feedback(&self, last_pts_us: u64, queue: u8) {
        self.viewer_queue.store(i32::from(queue), Ordering::Relaxed);
        let mut q = lock(&self.sent);
        while q.front().is_some_and(|p| *p <= last_pts_us) {
            q.pop_front();
        }
    }

    /// Yo'ldagi (tasdiqlanmagan) kadrlar soni.
    pub fn in_flight(&self) -> usize {
        lock(&self.sent).len()
    }

    /// Ruxsat etilgan yo'ldagi kadrlar: FPS x (RTT + javob oralig'i + navbat chegarasi).
    pub fn allowed_in_flight(&self) -> usize {
        let fps = f64::from(self.fps.load(Ordering::Relaxed));
        let rtt = Duration::from_micros(self.rtt_us.load(Ordering::Relaxed));
        let window = rtt + FEEDBACK_INTERVAL + TARGET_QUEUE;
        ((fps * window.as_secs_f64()).ceil() as usize).max(MIN_IN_FLIGHT)
    }

    /// Keyingi kadrni kodlamaslik kerakmi (tarmoq yoki Viewer ulgurmayapti).
    pub fn should_skip(&self) -> bool {
        self.in_flight() > self.allowed_in_flight() || self.viewer_queue.load(Ordering::Relaxed) >= VIEWER_QUEUE_LIMIT
    }
}

/// Viewer tomoni: qabul qilingan kadrlar, dekoder navbati va kalit kadr so'rovlari.
/// Kadr qabul qiluvchi vazifa, dekoder va control oqimiga yozuvchi sikl o'rtasida bo'lishiladi.
pub struct ViewerFeedback {
    last_pts: AtomicU64,
    /// Oxirgi `Feedback` dan keyin yangi kadr keldi.
    dirty: AtomicBool,
    queued: AtomicI32,
    want_key: AtomicBool,
    need_key: AtomicBool,
    last_key_request: Mutex<Option<Instant>>,
}

impl Default for ViewerFeedback {
    fn default() -> Self {
        ViewerFeedback {
            last_pts: AtomicU64::new(0),
            dirty: AtomicBool::new(false),
            queued: AtomicI32::new(0),
            want_key: AtomicBool::new(false),
            // Oqim boshida birinchi kalit kadrgacha hech narsa dekodlanmaydi
            need_key: AtomicBool::new(true),
            last_key_request: Mutex::new(None),
        }
    }
}

impl ViewerFeedback {
    /// Yangi ulanish uchun holatni tozalaydi.
    pub fn reset(&self) {
        self.last_pts.store(0, Ordering::Relaxed);
        self.dirty.store(false, Ordering::Relaxed);
        self.queued.store(0, Ordering::Relaxed);
        self.want_key.store(false, Ordering::Relaxed);
        self.need_key.store(true, Ordering::Relaxed);
        *lock(&self.last_key_request) = None;
    }

    /// Kadr tarmoqdan keldi (dekoderga berilishidan qat'i nazar: bu tarmoq tasdig'i).
    pub fn frame_received(&self, pts_us: u64) {
        self.last_pts.store(pts_us, Ordering::Relaxed);
        self.dirty.store(true, Ordering::Relaxed);
    }

    /// Kadrni dekoderga berish kerakmi: kalit kadr kutilayotgan bo'lsa, oddiy kadrlar tashlanadi.
    pub fn accept(&self, keyframe: bool) -> bool {
        if keyframe {
            self.need_key.store(false, Ordering::Relaxed);
            return true;
        }
        !self.need_key.load(Ordering::Relaxed)
    }

    /// Kadr dekoder navbatiga qo'yildi / navbatdan olindi.
    pub fn queued(&self) {
        self.queued.fetch_add(1, Ordering::Relaxed);
    }
    pub fn dequeued(&self) {
        // reset() dan keyin manfiyga tushmasin (kichik poyga zararsiz: faqat taxminiy navbat uzunligi)
        if self.queued.fetch_sub(1, Ordering::Relaxed) <= 0 {
            self.queued.store(0, Ordering::Relaxed);
        }
    }

    /// Zanjir uzildi (kadr tashlandi yoki dekodlash xatosi): keyingi kalit kadrgacha kadrlar tashlanadi va Host'dan
    /// kalit kadr so'raladi.
    pub fn request_keyframe(&self) {
        self.need_key.store(true, Ordering::Relaxed);
        self.want_key.store(true, Ordering::Relaxed);
    }

    /// Control oqimiga yuborish kerak bo'lgan `Feedback` (oxirgi yuborilgandan keyin yangi kadr kelgan bo'lsa).
    pub fn take_feedback(&self) -> Option<(u64, u8)> {
        if !self.dirty.swap(false, Ordering::Relaxed) {
            return None;
        }
        let q = self.queued.load(Ordering::Relaxed).clamp(0, 255) as u8;
        Some((self.last_pts.load(Ordering::Relaxed), q))
    }

    /// `RequestKeyframe` yuborish kerakmi (so'rovlar `KEYFRAME_REQUEST_GAP` dan tez-tez yuborilmaydi).
    pub fn take_keyframe_request(&self) -> bool {
        if !self.want_key.load(Ordering::Relaxed) {
            return false;
        }
        let mut last = lock(&self.last_key_request);
        if last.is_some_and(|t| t.elapsed() < KEYFRAME_REQUEST_GAP) {
            return false;
        }
        self.want_key.store(false, Ordering::Relaxed);
        *last = Some(Instant::now());
        true
    }
}

/// Ketma-ket sichqoncha harakatlaridan faqat oxirgisini qoldiradi (tugma, g'ildirak va klavishlar tartibi saqlanadi).
pub fn coalesce_inputs(events: impl IntoIterator<Item = InputEvent>) -> Vec<InputEvent> {
    let mut out: Vec<InputEvent> = Vec::new();
    for ev in events {
        if let (InputEvent::PointerMove { .. }, Some(InputEvent::PointerMove { .. })) = (ev, out.last()) {
            *out.last_mut().expect("bor") = ev;
        } else {
            out.push(ev);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_flight_is_cleared_by_feedback() {
        let c = VideoControl::new(30);
        for pts in [10, 20, 30, 40] {
            c.on_sent(pts);
        }
        assert_eq!(c.in_flight(), 4);
        c.on_feedback(25, 0);
        assert_eq!(c.in_flight(), 2);
        c.on_feedback(40, 0);
        assert_eq!(c.in_flight(), 0);
    }

    #[test]
    fn skips_when_too_many_frames_in_flight() {
        let c = VideoControl::new(30);
        c.set_rtt(Duration::from_millis(50));
        let allowed = c.allowed_in_flight();
        assert!(allowed >= MIN_IN_FLIGHT);
        for i in 0..allowed as u64 {
            c.on_sent(i);
        }
        assert!(!c.should_skip());
        c.on_sent(allowed as u64);
        assert!(c.should_skip());
        c.on_feedback(allowed as u64, 0);
        assert!(!c.should_skip());
    }

    #[test]
    fn skips_when_viewer_queue_is_full() {
        let c = VideoControl::new(30);
        c.on_feedback(0, VIEWER_QUEUE_LIMIT as u8);
        assert!(c.should_skip());
        c.on_feedback(0, 0);
        assert!(!c.should_skip());
    }

    #[test]
    fn keyframe_request_is_taken_once() {
        let c = VideoControl::new(30);
        assert!(!c.take_keyframe_request());
        c.request_keyframe();
        assert!(c.take_keyframe_request());
        assert!(!c.take_keyframe_request());
    }

    #[test]
    fn viewer_waits_for_keyframe_and_rate_limits_requests() {
        let f = ViewerFeedback::default();
        assert!(!f.accept(false), "oqim boshida kalit kadr kutiladi");
        assert!(f.accept(true));
        assert!(f.accept(false));
        f.request_keyframe();
        assert!(!f.accept(false));
        assert!(f.take_keyframe_request());
        f.request_keyframe();
        assert!(!f.take_keyframe_request(), "300 ms ichida qayta so'ralmaydi");
        assert!(f.accept(true));
    }

    #[test]
    fn feedback_only_after_new_frames() {
        let f = ViewerFeedback::default();
        assert_eq!(f.take_feedback(), None);
        f.frame_received(77);
        f.queued();
        assert_eq!(f.take_feedback(), Some((77, 1)));
        assert_eq!(f.take_feedback(), None);
        f.dequeued();
        f.dequeued();
        f.frame_received(78);
        assert_eq!(f.take_feedback(), Some((78, 0)));
    }

    #[test]
    fn coalesce_keeps_last_move_and_order() {
        use InputEvent::*;
        let evs = vec![
            PointerMove { x: 1, y: 1 },
            PointerMove { x: 2, y: 2 },
            Button { button: 0, pressed: true },
            PointerMove { x: 3, y: 3 },
            PointerMove { x: 4, y: 4 },
            Key { code: 30, pressed: true },
        ];
        assert_eq!(
            coalesce_inputs(evs),
            vec![
                PointerMove { x: 2, y: 2 },
                Button { button: 0, pressed: true },
                PointerMove { x: 4, y: 4 },
                Key { code: 30, pressed: true },
            ]
        );
    }
}
