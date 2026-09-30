use anyhow::Result;
use nvml_wrapper::Nvml;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::sync::Mutex;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ThermalStatus {
    pub gpu_temp_c: u32,
    pub gpu_utilization: u32,
    pub memory_used_mb: u32,
    pub memory_total_mb: u32,
    pub fan_speed: u32,
    pub power_draw_w: u32,
    pub power_limit_w: u32,
    pub throttling: bool,
    pub render_paused: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ThermalConfig {
    pub max_temp_c: u32,
    pub pause_temp_c: u32,
    pub resume_temp_c: u32,
    pub check_interval_sec: u64,
}

impl Default for ThermalConfig {
    fn default() -> Self {
        Self {
            max_temp_c: 85,
            pause_temp_c: 82,
            resume_temp_c: 75,
            check_interval_sec: 5,
        }
    }
}

lazy_static::lazy_static! {
    static ref THERMAL_STATE: Arc<Mutex<ThermalState>> = Arc::new(Mutex::new(ThermalState::default()));
    static ref THERMAL_CONFIG: Arc<Mutex<ThermalConfig>> = Arc::new(Mutex::new(ThermalConfig::default()));
}

#[derive(Default)]
struct ThermalState {
    current: Option<ThermalStatus>,
    render_paused: bool,
    #[allow(dead_code)] // Kept to prevent the JoinHandle from being dropped
    monitor_handle: Option<tokio::task::JoinHandle<()>>,
}


pub fn start_monitor(app_handle: AppHandle) {
    let state = THERMAL_STATE.clone();
    let config = THERMAL_CONFIG.clone();
    let app_handle_clone = app_handle.clone();
    
    tauri::async_runtime::spawn(async move {
        let nvml = match Nvml::init() {
            Ok(n) => n,
            Err(e) => {
                eprintln!("Failed to init NVML: {}", e);
                return;
            }
        };
        
        let device = match nvml.device_by_index(0) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Failed to get GPU device: {}", e);
                return;
            }
        };
        
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        
        loop {
            interval.tick().await;
            
            let cfg = config.lock().await.clone();
            
            let temp = device.temperature(nvml_wrapper::enum_wrappers::device::TemperatureSensor::Gpu).unwrap_or(0);
            
            // Use generic fallbacks for utilization and memory since types vary by version
            let utilization = device.utilization_rates().ok();
            let gpu_util = utilization.as_ref().map(|u| u.gpu).unwrap_or(0);
            
            let memory = device.memory_info().ok();
            let mem_used = memory.as_ref().map(|m| m.used).unwrap_or(0);
            let mem_total = memory.as_ref().map(|m| m.total).unwrap_or(0);
            
            let fan_speed = device.fan_speed(0).unwrap_or(0);
            let power = device.power_usage().unwrap_or(0) / 1000; // mW to W
            let power_limit = device.power_management_limit().unwrap_or(0) / 1000;
            
            let throttling = temp >= cfg.max_temp_c;
            let should_pause = temp >= cfg.pause_temp_c;
            let should_resume = temp <= cfg.resume_temp_c;
            
            let mut state_guard = state.lock().await;
            
            let was_paused = state_guard.render_paused;
            
            if should_pause && !was_paused {
                state_guard.render_paused = true;
                eprintln!("GPU temp {}°C >= {}°C, pausing renders", temp, cfg.pause_temp_c);
            } else if should_resume && was_paused {
                state_guard.render_paused = false;
                eprintln!("GPU temp {}°C <= {}°C, resuming renders", temp, cfg.resume_temp_c);
            }
            
            let status = ThermalStatus {
                gpu_temp_c: temp,
                gpu_utilization: gpu_util,
                memory_used_mb: (mem_used / 1024 / 1024) as u32,
                memory_total_mb: (mem_total / 1024 / 1024) as u32,
                fan_speed,
                power_draw_w: power,
                power_limit_w: power_limit,
                throttling,
                render_paused: state_guard.render_paused,
            };
            
            state_guard.current = Some(status.clone());
            
            // Emit to frontend
            let _ = app_handle_clone.emit("thermal-update", &status);
        }
    });
}

#[tauri::command]
pub async fn get_thermal_status() -> Result<ThermalStatus, String> {
    let state = THERMAL_STATE.lock().await;
    state.current.clone().ok_or("Thermal monitor not started".to_string())
}

#[tauri::command]
pub async fn get_thermal_config() -> Result<ThermalConfig, String> {
    let config = THERMAL_CONFIG.lock().await;
    Ok(config.clone())
}

#[tauri::command]
pub async fn set_thermal_config(config: ThermalConfig) -> Result<(), String> {
    let mut config_guard = THERMAL_CONFIG.lock().await;
    *config_guard = config;
    Ok(())
}

#[tauri::command]
pub async fn is_render_paused() -> Result<bool, String> {
    let state = THERMAL_STATE.lock().await;
    Ok(state.render_paused)
}

#[tauri::command]
pub async fn wait_for_thermal_clear(_app_handle: AppHandle) -> Result<(), String> {
    // Wait until render can proceed
    loop {
        let status = get_thermal_status().await?;
        if !status.render_paused {
            return Ok(());
        }
        
        // Wait for thermal update event
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}