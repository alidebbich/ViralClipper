use clipper_lib::run;

fn main() {
    // Note: logging is initialized by tauri-plugin-log in lib.rs.
    // All tracing macros (info!, warn!, error!) route through it.
    run();
}
