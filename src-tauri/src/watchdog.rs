use sysinfo::{System, Pid};
use tauri::{AppHandle, Emitter};
use std::time::Duration;
use tracing::{info, warn, error};

const MAX_RAM_PERCENT: f32 = 85.0;
const WARN_RAM_PERCENT: f32 = 80.0;

pub fn spawn_watchdog(app_handle: Option<AppHandle>) {
    tauri::async_runtime::spawn(async move {
        let mut sys = System::new_all();
        let current_pid = sysinfo::get_current_pid().expect("Failed to get current PID");
        
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            sys.refresh_memory();
            
            let total_ram = sys.total_memory() as f32;
            let used_ram = sys.used_memory() as f32;
            let percent_used = (used_ram / total_ram) * 100.0;
            
            if percent_used >= MAX_RAM_PERCENT {
                error!("CRITICAL: System RAM is at {:.1}%. Killing child processes to protect PC.", percent_used);
                sys.refresh_processes();
                
                // Kill all child process trees
                kill_child_processes(&sys, current_pid);
                
                // Alert UI
                let err_msg = format!("Stopped to protect your PC: RAM {:.0}%", percent_used);
                if let Some(app) = &app_handle {
                    let _ = app.emit("watchdog-alert", serde_json::json!({
                        "message": err_msg,
                        "level": "error"
                    }));
                    // Also abort pipelines
                    crate::pipeline::abort_all_pipelines(app).await;
                }
            } else if percent_used >= WARN_RAM_PERCENT {
                warn!("WARNING: System RAM is at {:.1}%.", percent_used);
                if let Some(app) = &app_handle {
                    let _ = app.emit("watchdog-alert", serde_json::json!({
                        "message": format!("High RAM usage: {:.0}%", percent_used),
                        "level": "warning"
                    }));
                }
            }
        }
    });
}

pub fn kill_child_processes(sys: &System, parent_pid: Pid) {
    for (pid, process) in sys.processes() {
        if process.parent() == Some(parent_pid) {
            let name = process.name().to_lowercase();
            if name.contains("python") || name.contains("ffmpeg") || name.contains("yt-dlp") {
                info!("Killing child worker process PID {} ({})", pid, name);
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/T", "/F"])
                    .output();
            }
        }
    }
}
