//! JNI sirt: Kotlin `com.remotedesktop.mobile.Native` obyekti. Matnli hodisalar `\t` bilan ajratiladi.

use crate::session::{self, Event, Params, Sess};
use jni::{
    objects::{JObject, JString},
    sys::{jboolean, jbyteArray, jint, jlongArray, jstring, JNI_FALSE, JNI_TRUE},
    JNIEnv,
};
use protocol::InputEvent;
use rdcore::{
    paths,
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

/// Ilovaning `files` papkasi: sozlamalar va known_hosts shu yerda saqlanadi.
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
    let Ok(known_hosts) = paths::known_hosts_file() else { return JNI_FALSE };
    let cfg = AgentConfig { server: cfg_server, key, name, id, token, known_hosts, host_port: None };
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
    let params = Params {
        id,
        password,
        fps: fps.clamp(1, 120) as u16,
        kbps: kbps.clamp(100, 100_000) as u32,
        height: height.clamp(0, 4320) as u16,
        connector,
    };
    let sess = session::start(params);
    if let Some(old) = lock(&SESSION).replace(sess) {
        old.close();
    }
    JNI_TRUE
}

/// Sessiya hodisasi: `confirm\tfp`, `connected\tfp\t0|1`, `video`, `ended\terror`; yo'q bo'lsa null.
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_pollEvent(mut env: JNIEnv, _o: JObject) -> jstring {
    let text = match current().and_then(|c| c.poll_event()) {
        Some(Event::Confirm(fp)) => format!("confirm\t{fp}"),
        Some(Event::Connected { fingerprint, via_tcp }) => format!("connected\t{fingerprint}\t{}", via_tcp as u8),
        Some(Event::VideoStarted) => "video".to_string(),
        Some(Event::Ended(e)) => format!("ended\t{}", e.unwrap_or_default()),
        None => return std::ptr::null_mut(),
    };
    jstr(&mut env, &text)
}

#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_confirm(_env: JNIEnv, _o: JObject, accept: jboolean) {
    if let Some(c) = current() {
        c.answer_confirm(accept != JNI_FALSE);
    }
}

/// Keyingi kadr: birinchi bayt 1 = kalit-kadr, qolgani Annex-B. Vaqt tugasa null.
#[no_mangle]
pub extern "system" fn Java_com_remotedesktop_mobile_Native_nextFrame(env: JNIEnv, _o: JObject, timeout_ms: jint) -> jbyteArray {
    let Some(c) = current() else { return std::ptr::null_mut() };
    let Some(f) = c.next_frame(Duration::from_millis(timeout_ms.max(1) as u64)) else { return std::ptr::null_mut() };
    let mut buf = Vec::with_capacity(f.data.len() + 1);
    buf.push(f.keyframe as u8);
    buf.extend_from_slice(&f.data);
    match env.byte_array_from_slice(&buf) {
        Ok(a) => a.into_raw(),
        Err(_) => std::ptr::null_mut(),
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
