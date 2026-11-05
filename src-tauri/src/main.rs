#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use clipper_lib::run;

fn main() {
    // Load .env in dev mode (never fails if file is missing)
    #[cfg(debug_assertions)]
    {
        let candidates = [
            std::path::PathBuf::from(".env"),
            std::path::PathBuf::from("../.env"),
        ];
        for env_path in &candidates {
            if env_path.exists() {
                if let Ok(content) = std::fs::read_to_string(env_path) {
                    for line in content.lines() {
                        let line = line.trim();
                        if line.is_empty() || line.starts_with('#') { continue; }
                        if let Some((k, v)) = line.split_once('=') {
                            let k = k.trim();
                            let v = v.trim().trim_matches('"').trim_matches('\'');
                            if std::env::var(k).is_err() {
                                std::env::set_var(k, v);
                            }
                        }
                    }
                }
                break;
            }
        }
    }

    run();
}
