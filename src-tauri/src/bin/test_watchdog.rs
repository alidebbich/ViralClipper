use std::process::{Command, Stdio};
use std::time::Instant;

#[tokio::main]
async fn main() {
    println!("Starting RAM Watchdog test...");
    
    // Spawn the watchdog without an AppHandle
    clipper_lib::watchdog::spawn_watchdog(None);
    
    // Start the ram_leaker.py
    println!("Spawning python tests/smoke/ram_leaker.py...");
    let mut child = Command::new("python")
        .args(["tests/smoke/ram_leaker.py"])
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("Failed to spawn ram_leaker.py");
        
    let start_time = Instant::now();
    
    // Wait for it to be killed
    let status = child.wait().expect("Failed to wait on child");
    let elapsed = start_time.elapsed();
    
    println!("Child process exited with status: {}", status);
    println!("Time taken: {:.2}s", elapsed.as_secs_f64());
    
    // If it was killed, it should have a non-zero exit code or signal
    assert!(!status.success(), "Process was supposed to be killed by watchdog but exited normally.");
    println!("Watchdog test SUCCESS. Process was killed.");
}
