#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(not(feature = "blank-baseline"))]
    if let Some(code) = marktub_lib::job_supervisor_entry() {
        std::process::exit(code);
    }
    marktub_lib::run();
}
