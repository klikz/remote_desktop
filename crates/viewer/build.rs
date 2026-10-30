//! Linux Host faylini viewer ichiga joylash: `RD_HOST_BIN=/yo'l/host-linux cargo build --release -p viewer`.
//! Berilmasa, viewer Host faylisiz yig'iladi (Linux'da "Ulanishga ruxsat" xizmatni o'rnata olmaydi).

use std::{env, fs, path::Path, path::PathBuf};

fn main() {
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
