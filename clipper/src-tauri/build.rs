// Build script for bundling binaries and resources

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Tell cargo to rerun if build.rs changes
    println!("cargo:rerun-if-changed=build.rs");
    
    // Set up paths
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target_dir = out_dir.join("bundled");
    std::fs::create_dir_all(&target_dir).unwrap();
    
    // On Windows, we could bundle ffmpeg and ollama
    // For now, we rely on system PATH
    #[cfg(target_os = "windows")]
    {
        // Check for ffmpeg in PATH
        if let Ok(output) = Command::new("where").arg("ffmpeg").output() {
            if output.status.success() {
                let ffmpeg_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                println!("cargo:rustc-env=CLIPPER_FFMPEG_PATH={}", ffmpeg_path);
            }
        }
        
        // Check for ollama in PATH
        if let Ok(output) = Command::new("where").arg("ollama").output() {
            if output.status.success() {
                let ollama_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                println!("cargo:rustc-env=CLIPPER_OLLAMA_PATH={}", ollama_path);
            }
        }
        
        // Check for python
        if let Ok(output) = Command::new("where").arg("python").output() {
            if output.status.success() {
                let python_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                println!("cargo:rustc-env=CLIPPER_PYTHON_PATH={}", python_path);
            }
        }
    }
    
    #[cfg(not(target_os = "windows"))]
    {
        // Check for ffmpeg in PATH
        if let Ok(output) = Command::new("which").arg("ffmpeg").output() {
            if output.status.success() {
                let ffmpeg_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                println!("cargo:rustc-env=CLIPPER_FFMPEG_PATH={}", ffmpeg_path);
            }
        }
        
        // Check for ollama
        if let Ok(output) = Command::new("which").arg("ollama").output() {
            if output.status.success() {
                let ollama_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                println!("cargo:rustc-env=CLIPPER_OLLAMA_PATH={}", ollama_path);
            }
        }
        
        // Check for python
        if let Ok(output) = Command::new("which").arg("python3").output() {
            if output.status.success() {
                let python_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                println!("cargo:rustc-env=CLIPPER_PYTHON_PATH={}", python_path);
            }
        }
    }
    
    // Copy Python scripts to output directory for bundling
    let python_src = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../python");
    let python_dst = target_dir.join("python");
    
    if python_src.exists() {
        copy_dir(&python_src, &python_dst).unwrap();
        println!("cargo:rustc-env=CLIPPER_PYTHON_SCRIPTS={}", python_dst.display());
    }
    
    // Copy assets
    let assets_src = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../assets");
    let assets_dst = target_dir.join("assets");
    
    if assets_src.exists() {
        copy_dir(&assets_src, &assets_dst).unwrap();
        println!("cargo:rustc-env=CLIPPER_ASSETS={}", assets_dst.display());
    }
}

fn copy_dir(src: &PathBuf, dst: &PathBuf) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        
        if src_path.is_dir() {
            copy_dir(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}