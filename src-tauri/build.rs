fn main() {
    // Copy sidecar binaries to target directory
    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let target_bin_dir = manifest_dir.join("binaries");
    
    if target_bin_dir.exists() {
        let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        
        for entry in std::fs::read_dir(&target_bin_dir).unwrap() {
            let entry = entry.unwrap();
            let dest = out_dir.join(entry.file_name());
            std::fs::copy(entry.path(), dest).ok();
        }
    }
    
    tauri_build::build()
}