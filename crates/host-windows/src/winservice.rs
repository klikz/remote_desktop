//! SYSTEM huquqli Windows xizmati: faol konsol sessiyasida `host-windows run` ni SYSTEM tokeni bilan
//! ishga tushiradi. Shunda host qulf ekrani va UAC (Winlogon desktop)ga ham ulana oladi (`desktop.rs`).
//!
//! O'rnatish (administrator): `host-windows install-service`; olib tashlash: `host-windows uninstall-service`.

use anyhow::{bail, Context, Result};
use std::{
    ffi::{c_void, OsString},
    process::Command,
    ptr,
    sync::mpsc,
    time::Duration,
};
use windows_service::{
    define_windows_service,
    service::{ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus, ServiceType},
    service_control_handler::{self, ServiceControlHandlerResult},
    service_dispatcher,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    Security::{DuplicateTokenEx, SecurityImpersonation, SetTokenInformation, TokenPrimary, TokenSessionId, TOKEN_ALL_ACCESS},
    System::{
        RemoteDesktop::WTSGetActiveConsoleSessionId,
        Threading::{
            CreateProcessAsUserW, GetCurrentProcess, GetExitCodeProcess, OpenProcessToken, TerminateProcess, CREATE_NO_WINDOW,
            PROCESS_INFORMATION, STARTUPINFOW,
        },
    },
};

pub const SERVICE_NAME: &str = "RemoteDesktopHost";
const NO_SESSION: u32 = 0xFFFF_FFFF;
const STILL_ACTIVE: u32 = 259;

pub fn install() -> Result<()> {
    let exe = std::env::current_exe()?;
    let cfg = rdcore::paths::config_dir()?;
    // Parol/sertifikat o'rnatayotgan foydalanuvchining papkasida qoladi (SYSTEM profilida emas)
    let bin = format!("\"{}\" service --config-dir \"{}\"", exe.display(), cfg.display());
    let out = Command::new("sc.exe")
        .args(["create", SERVICE_NAME, "binPath=", &bin, "start=", "auto", "obj=", "LocalSystem", "DisplayName=", "Remote Desktop Host"])
        .output()
        .context("sc.exe ishga tushmadi")?;
    if !out.status.success() {
        bail!("xizmat yaratilmadi (administrator huquqi kerak): {}", String::from_utf8_lossy(&out.stdout));
    }
    let _ = Command::new("sc.exe").args(["start", SERVICE_NAME]).output();
    println!("Xizmat o'rnatildi va ishga tushirildi. Konfiguratsiya papkasi: {}", cfg.display());
    println!("Parol o'rnatilmagan bo'lsa: `host-windows set-password` (shu foydalanuvchi nomidan).");
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let _ = Command::new("sc.exe").args(["stop", SERVICE_NAME]).output();
    let out = Command::new("sc.exe").args(["delete", SERVICE_NAME]).output().context("sc.exe ishga tushmadi")?;
    if !out.status.success() {
        bail!("xizmat o'chirilmadi: {}", String::from_utf8_lossy(&out.stdout));
    }
    println!("Xizmat o'chirildi.");
    Ok(())
}

/// SCM kirish nuqtasi (`host-windows service ...`).
pub fn run_service(args: &[String]) -> Result<()> {
    if let Some(i) = args.iter().position(|a| a == "--config-dir") {
        if let Some(d) = args.get(i + 1) {
            std::env::set_var("RD_CONFIG_DIR", d); // bola jarayon shu muhitni meros oladi
        }
    }
    service_dispatcher::start(SERVICE_NAME, ffi_service_main).context("xizmat dispetcheri ishga tushmadi (SCM'dan emas)")?;
    Ok(())
}

define_windows_service!(ffi_service_main, service_main);

fn service_main(_args: Vec<OsString>) {
    let _ = run_supervisor();
}

fn status(state: ServiceState, accept: ServiceControlAccept) -> ServiceStatus {
    ServiceStatus {
        service_type: ServiceType::OWN_PROCESS,
        current_state: state,
        controls_accepted: accept,
        exit_code: ServiceExitCode::Win32(0),
        checkpoint: 0,
        wait_hint: Duration::from_secs(5),
        process_id: None,
    }
}

fn run_supervisor() -> Result<()> {
    let (tx, rx) = mpsc::channel::<()>();
    let handle = service_control_handler::register(SERVICE_NAME, move |c| match c {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;
    handle.set_service_status(status(ServiceState::Running, ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN))?;

    let exe = std::env::current_exe()?;
    let mut child: Option<(HANDLE, u32)> = None; // (jarayon, sessiya)
    loop {
        let sid = unsafe { WTSGetActiveConsoleSessionId() };
        if let Some((h, s)) = child {
            let mut code = 0u32;
            let alive = unsafe { GetExitCodeProcess(h, &mut code) } != 0 && code == STILL_ACTIVE;
            if !alive || s != sid {
                kill(h);
                child = None;
            }
        }
        if child.is_none() && sid != NO_SESSION {
            match spawn_in_session(&exe, sid) {
                Ok(h) => child = Some((h, sid)),
                Err(e) => eprintln!("[xizmat] host ishga tushmadi: {e:#}"),
            }
        }
        if rx.recv_timeout(Duration::from_secs(2)).is_ok() {
            break;
        }
    }
    if let Some((h, _)) = child {
        kill(h);
    }
    handle.set_service_status(status(ServiceState::Stopped, ServiceControlAccept::empty()))?;
    Ok(())
}

fn kill(h: HANDLE) {
    unsafe {
        TerminateProcess(h, 0);
        CloseHandle(h);
    }
}

/// O'z SYSTEM tokenining nusxasini `sid` sessiyasiga qo'yib, `host-windows run` ni shu yerda ishga tushiradi.
fn spawn_in_session(exe: &std::path::Path, sid: u32) -> Result<HANDLE> {
    unsafe {
        let mut own: HANDLE = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut own) == 0 {
            bail!("OpenProcessToken xatosi");
        }
        let mut tok: HANDLE = ptr::null_mut();
        let ok = DuplicateTokenEx(own, TOKEN_ALL_ACCESS, ptr::null(), SecurityImpersonation, TokenPrimary, &mut tok);
        CloseHandle(own);
        if ok == 0 {
            bail!("DuplicateTokenEx xatosi");
        }
        if SetTokenInformation(tok, TokenSessionId, &sid as *const u32 as *const c_void, 4) == 0 {
            CloseHandle(tok);
            bail!("SetTokenInformation(sessiya) xatosi");
        }
        let mut cmd: Vec<u16> = format!("\"{}\" run", exe.display()).encode_utf16().chain(Some(0)).collect();
        let mut desktop: Vec<u16> = "winsta0\\default".encode_utf16().chain(Some(0)).collect();
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        si.lpDesktop = desktop.as_mut_ptr();
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessAsUserW(tok, ptr::null(), cmd.as_mut_ptr(), ptr::null(), ptr::null(), 0, CREATE_NO_WINDOW, ptr::null(), ptr::null(), &si, &mut pi);
        CloseHandle(tok);
        if ok == 0 {
            bail!("CreateProcessAsUserW xatosi");
        }
        CloseHandle(pi.hThread);
        Ok(pi.hProcess)
    }
}
