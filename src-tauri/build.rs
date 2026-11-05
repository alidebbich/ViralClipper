fn main() {
    // Copy sidecar binaries to OUT_DIR with target triple naming (what Tauri expects)
    let manifest_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let target_bin_dir = manifest_dir.join("binaries");
    
    if target_bin_dir.exists() {
        let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
        let target_triple = std::env::var("TARGET").unwrap_or_else(|_| "x86_64-pc-windows-msvc".to_string());
        
        for entry in std::fs::read_dir(&target_bin_dir).unwrap() {
            let entry = entry.unwrap();
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();
            
            // Create platform-specific name with target triple (what tauri expects)
            let dest_name = if file_name_str.ends_with(".exe") {
                file_name_str.replace(".exe", &format!("-{}.exe", target_triple))
            } else if file_name_str.ends_with(".bat") {
                file_name_str.replace(".bat", &format!("-{}.bat", target_triple))
            } else {
                format!("{}-{}", file_name_str, target_triple)
            };
            
            let dest = out_dir.join(&dest_name);
            std::fs::copy(entry.path(), &dest).ok();
            
            // Also copy with original name for good measure
            let dest_orig = out_dir.join(file_name_str.as_ref());
            std::fs::copy(entry.path(), dest_orig).ok();
        }
    }
    
    tauri_build::build()
}