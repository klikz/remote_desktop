//! Linux: Host'ni dastur ichidan o'rnatish. Viewer ichiga joylangan Host fayli (`build.rs`) chiqariladi,
//! kerakli paketlar o'rnatiladi, GNOME qulf/uyqu o'chiriladi va systemd user xizmati yoqiladi.
//! Jarayon alohida oqimda ishlaydi va qadamlarni `Msg` orqali xabar qiladi.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::Command,
    sync::mpsc::{channel, Receiver, Sender},
};

include!(concat!(env!("OUT_DIR"), "/embedded_host.rs"));

const PKGS: [&str; 9] = [
    "gstreamer1.0-tools",
    "gstreamer1.0-pipewire",
    "gstreamer1.0-plugins-base",
    "gstreamer1.0-plugins-good",
    "gstreamer1.0-plugins-bad",
    "gstreamer1.0-plugins-ugly",
    "pipewire",
    "xdg-desktop-portal",
    "xdg-desktop-portal-gnome",
];

// schema | kalit | qiymat
const SETTINGS: [(&str, &str, &str); 5] = [
    ("org.gnome.desktop.screensaver", "lock-enabled", "false"),
    ("org.gnome.desktop.session", "idle-delay", "0"),
    ("org.gnome.desktop.lockdown", "disable-lock-screen", "true"),
    ("org.gnome.settings-daemon.plugins.power", "sleep-inactive-ac-type", "'nothing'"),
    ("org.gnome.settings-daemon.plugins.power", "sleep-inactive-battery-type", "'nothing'"),
];

const GDM_CONF: &str = "/etc/gdm3/custom.conf";

const GDM_PY: &str = r#"
import sys, re
path, user = sys.argv[1], sys.argv[2]
lines = open(path).read().split('\n')
out, in_daemon, done = [], False, set()
for l in lines:
    s = l.strip()
    if s.startswith('['):
        if in_daemon:
            for k, v in (('AutomaticLoginEnable', 'true'), ('AutomaticLogin', user)):
                if k not in done: out.append(f'{k}={v}'); done.add(k)
        in_daemon = (s == '[daemon]')
        out.append(l); continue
    if in_daemon:
        m = re.match(r'#?\s*(AutomaticLoginEnable|AutomaticLogin)\s*=', s)
        if m:
            k = m.group(1)
            if k not in done:
                out.append(f'{k}=' + ('true' if k == 'AutomaticLoginEnable' else user)); done.add(k)
            continue
    out.append(l)
if in_daemon:
    for k, v in (('AutomaticLoginEnable', 'true'), ('AutomaticLogin', user)):
        if k not in done: out.append(f'{k}={v}'); done.add(k)
if not any(l.strip() == '[daemon]' for l in lines):
    out += ['[daemon]', 'AutomaticLoginEnable=true', f'AutomaticLogin={user}']
open(path, 'w').write('\n'.join(out))
"#;

pub enum Msg {
    Step(String),
    Done(Result<(), String>),
}

/// Viewer ichiga Host fayli joylanganmi.
pub fn embedded_available() -> bool {
    HOST_BIN.is_some()
}

fn home() -> Result<PathBuf, String> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| "HOME aniqlanmadi".to_string())
}

pub fn bin_path() -> Option<PathBuf> {
    home().ok().map(|h| h.join(".local/bin/remote-desktop-host"))
}

fn config_home() -> Result<PathBuf, String> {
    match std::env::var_os("XDG_CONFIG_HOME") {
        Some(x) if !x.is_empty() => Ok(PathBuf::from(x)),
        _ => Ok(home()?.join(".config")),
    }
}

/// O'rnatilgan Host fayli viewer ichidagi bilan bir xilmi (yangilash kerakmi?).
pub fn installed_is_current() -> bool {
    let (Some(embedded), Some(path)) = (HOST_BIN, bin_path()) else { return true };
    fs::read(path).map(|b| b == embedded).unwrap_or(false)
}

/// GDM avtomatik kirish shu foydalanuvchi uchun yoqilganmi (`/etc/gdm3/custom.conf` hamma uchun o'qiladi).
pub fn autologin_enabled() -> bool {
    let Ok(user) = std::env::var("USER") else { return false };
    let Ok(text) = fs::read_to_string(GDM_CONF) else { return false };
    let mut enable = false;
    let mut who = false;
    for l in text.lines().map(str::trim) {
        if let Some(v) = l.strip_prefix("AutomaticLoginEnable") {
            let v = v.trim_start().trim_start_matches('=').trim();
            enable = matches!(v, "true" | "True" | "1");
        } else if let Some(v) = l.strip_prefix("AutomaticLogin") {
            who = v.trim_start().trim_start_matches('=').trim() == user;
        }
    }
    enable && who
}

/// Host xizmatining oxirgi log qatorlari (xato sababini ko'rsatish uchun).
pub fn journal_tail(n: usize) -> String {
    Command::new("journalctl")
        .args(["--user", "-u", "remote-desktop-host", "-n", &n.to_string(), "--no-pager", "-o", "cat"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Xizmatni (o'rnatilgan bo'lsa) qayta ishga tushiradi va ishlayotganini tekshiradi.
pub fn spawn_start() -> Receiver<Msg> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        step(&tx, "Host ishga tushirilmoqda");
        let r = start_service();
        let _ = tx.send(Msg::Done(r));
    });
    rx
}

/// Faqat avtomatik kirishni yoqadi.
pub fn spawn_autologin() -> Receiver<Msg> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        step(&tx, "Avtomatik kirish yoqilmoqda (parol so'raladi)");
        let r = enable_autologin();
        let _ = tx.send(Msg::Done(r));
    });
    rx
}

fn enable_autologin() -> Result<(), String> {
    let user = std::env::var("USER").map_err(|_| "USER aniqlanmadi".to_string())?;
    let st = Command::new("pkexec")
        .args(["python3", "-c", GDM_PY, GDM_CONF, &user])
        .status()
        .map_err(|e| format!("pkexec ishga tushmadi: {e}"))?;
    if !st.success() {
        return Err("Avtomatik kirishni yoqib bo'lmadi (parol bekor qilingan bo'lishi mumkin)".into());
    }
    Ok(())
}

/// `reset-failed` + `enable` + `restart`, so'ng bir necha soniya kutib holatni tekshiradi.
fn start_service() -> Result<(), String> {
    let _ = Command::new("systemctl").args(["--user", "reset-failed", "remote-desktop-host"]).output();
    for args in [
        &["--user", "daemon-reload"][..],
        &["--user", "enable", "remote-desktop-host"][..],
        &["--user", "restart", "remote-desktop-host"][..],
    ] {
        if !ok(Command::new("systemctl").args(args)) {
            return Err(format!("systemctl {} bajarilmadi\n{}", args.join(" "), journal_tail(15)));
        }
    }
    std::thread::sleep(std::time::Duration::from_secs(3));
    let state = Command::new("systemctl")
        .args(["--user", "is-active", "remote-desktop-host"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    if state == "active" {
        Ok(())
    } else {
        Err(format!("Host ishga tushmadi (holat: {state}). Log:\n{}", journal_tail(15)))
    }
}

/// O'rnatishni alohida oqimda boshlaydi.
pub fn spawn_install(autologin: bool) -> Receiver<Msg> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let r = run(&tx, autologin);
        let _ = tx.send(Msg::Done(r));
    });
    rx
}

fn step(tx: &Sender<Msg>, text: impl Into<String>) {
    let _ = tx.send(Msg::Step(text.into()));
}

fn ok(cmd: &mut Command) -> bool {
    cmd.output().map(|o| o.status.success()).unwrap_or(false)
}

fn run(tx: &Sender<Msg>, autologin: bool) -> Result<(), String> {
    let data = HOST_BIN.ok_or("Bu dastur Host faylisiz yig'ilgan (RD_HOST_BIN berilmagan)")?;
    let password_file = rdcore::paths::host_password_file().map_err(|e| format!("{e:#}"))?;
    if !password_file.exists() {
        return Err("Avval ulanish parolini o'rnating".into());
    }

    step(tx, "1/5  Kerakli paketlar tekshirilmoqda");
    let missing: Vec<&str> = PKGS
        .iter()
        .copied()
        .filter(|p| !ok(Command::new("dpkg").args(["-s", p])))
        .collect();
    if !missing.is_empty() {
        step(tx, format!("1/5  Paketlar o'rnatilmoqda: {} (parol so'raladi)", missing.join(" ")));
        let script = format!("apt-get update && apt-get install -y {}", missing.join(" "));
        let st = Command::new("pkexec")
            .args(["sh", "-c", &script])
            .status()
            .map_err(|e| format!("pkexec ishga tushmadi: {e}"))?;
        if !st.success() {
            return Err("Paketlarni o'rnatib bo'lmadi (parol bekor qilingan bo'lishi mumkin)".into());
        }
    }

    step(tx, "2/5  Host fayli o'rnatilmoqda");
    let bin = bin_path().ok_or("HOME aniqlanmadi")?;
    fs::create_dir_all(bin.parent().ok_or("yo'l noto'g'ri")?).map_err(|e| format!("papka yaratilmadi: {e}"))?;
    let tmp = bin.with_extension("new");
    fs::write(&tmp, data).map_err(|e| format!("fayl yozilmadi: {e}"))?;
    fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755)).map_err(|e| format!("ruxsat berilmadi: {e}"))?;
    fs::rename(&tmp, &bin).map_err(|e| format!("fayl almashtirilmadi: {e}"))?;

    step(tx, "3/5  GNOME sozlamalari (qulf, ekran o'chishi, uyqu)");
    apply_gnome()?;

    if autologin && !autologin_enabled() {
        step(tx, "4/5  Avtomatik kirish yoqilmoqda (parol so'raladi)");
        enable_autologin()?;
    } else {
        step(tx, "4/5  Avtomatik kirish o'tkazib yuborildi");
    }

    step(tx, "5/5  Xizmat yoqilmoqda");
    let unit_dir = config_home()?.join("systemd/user");
    fs::create_dir_all(&unit_dir).map_err(|e| format!("papka yaratilmadi: {e}"))?;
    let unit = format!(
        "[Unit]\nDescription=Remote Desktop Host\nAfter=graphical-session.target\nPartOf=graphical-session.target\n\n\
         [Service]\nExecStart={} run\nRestart=always\nRestartSec=5\n\n[Install]\nWantedBy=graphical-session.target\n",
        bin.display()
    );
    fs::write(unit_dir.join("remote-desktop-host.service"), unit).map_err(|e| format!("xizmat fayli yozilmadi: {e}"))?;
    start_service()
}

fn have_key(schema: &str, key: &str) -> bool {
    ok(Command::new("gsettings").args(["get", schema, key]))
}

/// GNOME qulf/uyqu/ekran o'chishini o'chiradi. Asl qiymatlar zaxira faylga yoziladi (birinchi marta).
fn apply_gnome() -> Result<(), String> {
    let conf = rdcore::paths::config_dir().map_err(|e| format!("{e:#}"))?;
    fs::create_dir_all(&conf).map_err(|e| format!("papka yaratilmadi: {e}"))?;
    let backup = conf.join("gnome-settings.backup");
    if !backup.exists() {
        let mut text = String::new();
        for (schema, key, _) in SETTINGS {
            if let Ok(o) = Command::new("gsettings").args(["get", schema, key]).output() {
                if o.status.success() {
                    text.push_str(&format!("{schema}|{key}|{}\n", String::from_utf8_lossy(&o.stdout).trim()));
                }
            }
        }
        let _ = fs::write(&backup, text);
    }
    for (schema, key, value) in SETTINGS {
        if have_key(schema, key) {
            let _ = Command::new("gsettings").args(["set", schema, key, value]).status();
        }
    }
    Ok(())
}
