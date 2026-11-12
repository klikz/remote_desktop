//! JNI sirt: Kotlin `com.remotedesktop.mobile.Native` obyekti. Matnli hodisalar `\t` bilan ajratiladi.

use crate::session::{self, Event, Params, Sess};
use jni::{
    objects::{JByteBuffer, JObject, JString},
    sys::{jboolean, jint, jlong, jlongArray, jstring, JNI_FALSE, JNI_TRUE},
    JNIEnv,
};
use protocol::InputEvent;
use rdcore::{
    relay::agent::{Agent, AgentConfig, AgentEvent, Policy},
};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

static AGENT: Mutex<Option<Agent>> = Mutex::new(None);
static SESSION: Mutex<Option<Arc<Sess>>> = Mutex::new(None);

fn lock<T>(m: &'static Mutex<T>) -> std::sync::MutexGuard<'static, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn s(env: &mut JNIEnv, v: &JString) -> String {
    env.get_string(v).map(|j| j.into()).unwrap_or_default()
}

fn jstr(env: &mut JNIEnv, v: &str) -> jstring {
    env.new_string(v).map(|j| j.into_raw()).unwrap_or(std::ptr::null_mut())
}

fn current() -> Option<Arc<Sess>> {
    lock(&SESSION).clone()
}

/// Ilovaning `files` papkasi: sozlamalar shu yerda saqlanadi.
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_init(mut env: JNIEnv, _o: JObject, files_dir: JString) {
    let dir = s(&mut env, &files_dir);
    let _ = std::fs::create_dir_all(&dir);
    std::env::set_var("RD_CONFIG_DIR", dir);
}

/// Relay agentini ishga tushiradi (eskisi to'xtatiladi). `host:port` ko'rinishidagi `server`.
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_startAgent(
    mut env: JNIEnv,
    _o: JObject,
    server: JString,
    key: JString,
    name: JString,
    id: JString,
    token: JString,
) -> jboolean {
    let cfg_server = s(&mut env, &server);
    let key = s(&mut env, &key);
    let name = s(&mut env, &name);
    let id = s(&mut env, &id);
    let token = s(&mut env, &token);
    let cfg = AgentConfig { server: cfg_server, key, name, id, token, host_port: None };
    *lock(&AGENT) = Some(Agent::start(cfg, Policy::default()));
    JNI_TRUE
}

#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_stopAgent(_env: JNIEnv, _o: JObject) {
    *lock(&AGENT) = None;
}

/// Kontaktlar onlayn holatini kuzatish: ID'lar `,` bilan.
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_subscribe(mut env: JNIEnv, _o: JObject, ids: JString) {
    let ids: Vec<String> = s(&mut env, &ids).split(',').filter(|x| !x.is_empty()).map(String::from).collect();
    if let Some(a) = lock(&AGENT).as_ref() {
        a.subscribe(ids);
    }
}

/// Agent hodisasi: `status\t0|1\ttext`, `registered\tid\ttoken`, `presence\tid\t0|1\tname`; yo'q bo'lsa null.
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_agentEvent(mut env: JNIEnv, _o: JObject) -> jstring {
    let ev = lock(&AGENT).as_ref().and_then(|a| a.try_event());
    let text = match ev {
        Some(AgentEvent::Status { online, text }) => format!("status\t{}\t{}", online as u8, text),
        Some(AgentEvent::Registered { id, token }) => format!("registered\t{id}\t{token}"),
        Some(AgentEvent::Presence { id, online, name }) => format!("presence\t{id}\t{}\t{name}", online as u8),
        // Bu client faqat ulanadi: kiruvchi so'rovlar rad etiladi.
        Some(AgentEvent::Incoming { sid, .. }) => {
            if let Some(a) = lock(&AGENT).as_ref() {
                a.reply(&sid, false);
            }
            return std::ptr::null_mut();
        }
        None => return std::ptr::null_mut(),
    };
    jstr(&mut env, &text)
}

/// Sessiyani boshlaydi. `false`: agent ishlamayapti.
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_connect(
    mut env: JNIEnv,
    _o: JObject,
    id: JString,
    password: JString,
    fps: jint,
    kbps: jint,
    height: jint,
) -> jboolean {
    let id = s(&mut env, &id);
    let password = s(&mut env, &password);
    let Some(connector) = lock(&AGENT).as_ref().map(|a| a.connector()) else { return JNI_FALSE };
    start_session(Params {
        id,
        addr: String::new(),
        password,
        fps: fps.clamp(1, 120) as u16,
        kbps: kbps.clamp(100, 100_000) as u32,
        height: height.clamp(0, 4320) as u16,
        connector: Some(connector),
    })
}

fn start_session(params: Params) -> jboolean {
    let sess = session::start(params);
    if let Some(old) = lock(&SESSION).replace(sess) {
        old.close();
    }
    JNI_TRUE
}

/// P2P (to'g'ridan-to'g'ri) sessiya: `addr` = `IP`, `IP:port` yoki `nom[:port]`. Agent shart emas.
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_connectDirect(
    mut env: JNIEnv,
    _o: JObject,
    addr: JString,
    password: JString,
    fps: jint,
    kbps: jint,
    height: jint,
) -> jboolean {
    let addr = s(&mut env, &addr);
    let password = s(&mut env, &password);
    start_session(Params {
        id: String::new(),
        addr,
        password,
        fps: fps.clamp(1, 120) as u16,
        kbps: kbps.clamp(100, 100_000) as u32,
        height: height.clamp(0, 4320) as u16,
        connector: None,
    })
}

/// Qurilma tarmog'i o'zgardi (Wi-Fi <-> mobil internet va h.k.): agent va ochiq sessiya darrov qayta ulanadi.
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_networkChanged(_env: JNIEnv, _o: JObject) {
    if let Some(a) = lock(&AGENT).as_ref() {
        a.network_changed();
    }
    if let Some(c) = current() {
        c.network_changed();
    }
}

/// Sessiya hodisasi: `connected\t0|1`, `video`, `reconnecting\tattempt\treason`, `ended\terror`;
/// yo'q bo'lsa null.
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_pollEvent(mut env: JNIEnv, _o: JObject) -> jstring {
    let text = match current().and_then(|c| c.poll_event()) {
        Some(Event::Connected { via_tcp }) => format!("connected\t{}", via_tcp as u8),
        Some(Event::VideoStarted) => "video".to_string(),
        Some(Event::Reconnecting { attempt, reason }) => format!("reconnecting\t{attempt}\t{}", reason.replace('\t', " ")),
        Some(Event::Ended(e)) => format!("ended\t{}", e.unwrap_or_default()),
        None => return std::ptr::null_mut(),
    };
    jstr(&mut env, &text)
}

/// Keyingi kadrni kutadi: `-1` = vaqt tugadi, aks holda `(baytlar << 1) | kalit_kadr`. Kadr `copyFrame` yoki
/// `discardFrame` gacha Rust tomonda turadi (Java'da massiv yaratilmaydi).
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_waitFrame(_env: JNIEnv, _o: JObject, timeout_ms: jint) -> jlong {
    let Some(c) = current() else { return -1 };
    match c.wait_frame(Duration::from_millis(timeout_ms.max(1) as u64)) {
        Some((len, key)) => ((len as jlong) << 1) | jlong::from(key),
        None => -1,
    }
}

/// Kutilgan kadrni MediaCodec'ning kirish buferiga (direct `ByteBuffer`) to'g'ridan-to'g'ri ko'chiradi: yagona nusxa.
/// Natija: baytlar soni yoki `-1` (bufer sig'madi / kadr yo'q: kadr tashlandi, kalit kadr so'raldi).
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_copyFrame(env: JNIEnv, _o: JObject, buf: JByteBuffer) -> jint {
    let Some(c) = current() else { return -1 };
    let (Ok(ptr), Ok(cap)) = (env.get_direct_buffer_address(&buf), env.get_direct_buffer_capacity(&buf)) else {
        c.discard_frame(true);
        return -1;
    };
    if ptr.is_null() {
        c.discard_frame(true);
        return -1;
    }
    // SAFETY: JVM bergan direct bufer manzili va sig'imi; bufer shu chaqiruv davomida Java tomonda ushlab turiladi.
    let dst = unsafe { std::slice::from_raw_parts_mut(ptr, cap) };
    c.copy_frame(dst).map_or(-1, |n| n as jint)
}

/// Kutilgan kadrni tashlaydi. `chainBroken`: dekoder kadrni ololmadi, kalit kadr so'raladi.
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_discardFrame(_env: JNIEnv, _o: JObject, chain_broken: jboolean) {
    if let Some(c) = current() {
        c.discard_frame(chain_broken != JNI_FALSE);
    }
}

/// Dekoder xatosi yoki qayta yaratildi: Host'dan kalit kadr so'raladi.
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_requestKeyframe(_env: JNIEnv, _o: JObject) {
    if let Some(c) = current() {
        c.request_keyframe();
    }
}

/// `[kadrlar, baytlar, rtt_us]`
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_stats(env: JNIEnv, _o: JObject) -> jlongArray {
    use std::sync::atomic::Ordering::Relaxed;
    let Some(c) = current() else { return std::ptr::null_mut() };
    let v = [c.stats.frames.load(Relaxed) as i64, c.stats.bytes.load(Relaxed) as i64, c.stats.rtt_us.load(Relaxed) as i64];
    let Ok(arr) = env.new_long_array(3) else { return std::ptr::null_mut() };
    let _ = env.set_long_array_region(&arr, 0, &v);
    arr.into_raw()
}

fn send(ev: InputEvent) {
    if let Some(c) = current() {
        c.input(ev);
    }
}

/// `x`, `y`: 0..65535 (ko'rinadigan ekran kasri).
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_sendMove(_env: JNIEnv, _o: JObject, x: jint, y: jint) {
    send(InputEvent::PointerMove { x: x.clamp(0, 65535) as u16, y: y.clamp(0, 65535) as u16 });
}

/// `button`: 0 chap, 1 o'ng, 2 o'rta.
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_sendButton(_env: JNIEnv, _o: JObject, button: jint, pressed: jboolean) {
    send(InputEvent::Button { button: button.clamp(0, 2) as u8, pressed: pressed != JNI_FALSE });
}

/// Musbat = pastga/o'ngga (g'ildirak tishlari).
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_sendScroll(_env: JNIEnv, _o: JObject, dx: jint, dy: jint) {
    send(InputEvent::Scroll { dx: dx.clamp(-32768, 32767) as i16, dy: dy.clamp(-32768, 32767) as i16 });
}

/// `code`: Linux evdev klavish kodi.
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_sendKey(_env: JNIEnv, _o: JObject, code: jint, pressed: jboolean) {
    send(InputEvent::Key { code: code as u32, pressed: pressed != JNI_FALSE });
}

#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_close(_env: JNIEnv, _o: JObject) {
    if let Some(c) = lock(&SESSION).take() {
        c.close();
    }
}
