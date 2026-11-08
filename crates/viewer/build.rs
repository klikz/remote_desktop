//! Linux Host faylini viewer ichiga joylash: `RD_HOST_BIN=/yo'l/host-linux cargo build --release -p viewer`.
//! Berilmasa, viewer Host faylisiz yig'iladi (Linux'da "Ulanishga ruxsat" xizmatni o'rnata olmaydi).
//! Windows: exe ikonkasi (`assets/logo/app.ico` resurs sifatida qo'shiladi).

use std::{env, fs, path::Path, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=../../assets/logo/app.ico");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
        fs::copy("../../assets/logo/app.ico", dir.join("app.ico")).expect("app.ico ko'chirilmadi");
        let rc = dir.join("app.rc");
        let rc_text = "1 ICON \"app.ico\"\n1 VERSIONINFO\nFILEVERSION 0,1,0,0\nPRODUCTVERSION 0,1,0,0\nBEGIN\n BLOCK \"StringFileInfo\"\n BEGIN\n  BLOCK \"040904b0\"\n  BEGIN\n   VALUE \"ProductName\", \"Ulan\"\n   VALUE \"FileDescription\", \"Ulan\"\n   VALUE \"OriginalFilename\", \"ulan_desk.exe\"\n  END\n END\n BLOCK \"VarFileInfo\"\n BEGIN\n  VALUE \"Translation\", 0x409, 1200\n END\nEND\n";
        fs::write(&rc, rc_text).expect("app.rc yozilmadi");
        embed_resource::compile(&rc, embed_resource::NONE);
    }

    println!("cargo:rerun-if-env-changed=RD_HOST_BIN");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("embedded_host.rs");
    let code = match env::var("RD_HOST_BIN") {
        Ok(p) if !p.is_empty() && Path::new(&p).is_file() => {
            let abs = fs::canonicalize(&p).expect("RD_HOST_BIN yo'li");
            println!("cargo:rerun-if-changed={}", abs.display());
            format!("pub static HOST_BIN: Option<&[u8]> = Some(include_bytes!({:?}));\n", abs.display().to_string())
        }
        _ => "pub static HOST_BIN: Option<&[u8]> = None;\n".to_string(),
    };
    fs::write(out, code).expect("embedded_host.rs yozilmadi");
}
