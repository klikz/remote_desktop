//! Server portini xavfsizlik devorida (firewall) ochish. Windows'da UAC, Linux'da pkexec so'raydi.

#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::Command;

/// `port` raqamli UDP kiruvchi ulanishlarga ruxsat beradi. Muvaffaqiyatda qisqa xabar qaytaradi.
pub fn open_udp(port: u16) -> Result<String, String> {
    #[cfg(windows)]
    {
        const NO_WINDOW: u32 = 0x0800_0000;
        let rule = format!("Remote Desktop Relay {port}");
        let script = format!(
            "netsh advfirewall firewall delete rule name=\\\"{rule}\\\"; \
             netsh advfirewall firewall add rule name=\\\"{rule}\\\" dir=in action=allow protocol=UDP localport={port}"
        );
        let outer = format!(
            "Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-Command','{script}'"
        );
        let started = Command::new("powershell")
            .args(["-NoProfile", "-Command", &outer])
            .creation_flags(NO_WINDOW)
            .output()
            .map_err(|e| format!("powershell ishga tushmadi: {e}"))?;
        // UAC bekor qilinsa Start-Process xato beradi; qoida bor-yo'qligini alohida tekshiramiz
        let exists = Command::new("netsh")
            .args(["advfirewall", "firewall", "show", "rule", &format!("name={rule}")])
            .creation_flags(NO_WINDOW)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if exists {
            Ok(format!("Firewall: UDP {port} ochildi"))
        } else {
            let _ = started;
            Err("Firewall qoidasi qo'shilmadi (administrator ruxsati bekor qilingan bo'lishi mumkin)".into())
        }
    }
    #[cfg(not(windows))]
    {
        let script = format!(
            "if command -v ufw >/dev/null 2>&1 && ufw status | grep -q 'Status: active'; then ufw allow {port}/udp; else echo NO_UFW; fi"
        );
        let out = Command::new("pkexec")
            .args(["sh", "-c", &script])
            .output()
            .map_err(|e| format!("pkexec ishga tushmadi: {e}"))?;
        if !out.status.success() {
            return Err("ufw qoidasi qo'shilmadi (parol bekor qilingan bo'lishi mumkin)".into());
        }
        if String::from_utf8_lossy(&out.stdout).contains("NO_UFW") {
            Ok("ufw faol emas: port ochish shart emas".into())
        } else {
            Ok(format!("ufw: UDP {port} ochildi"))
        }
    }
}
