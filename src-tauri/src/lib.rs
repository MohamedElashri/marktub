#[cfg(not(feature = "blank-baseline"))]
mod app;
#[cfg(not(feature = "blank-baseline"))]
mod atomic_save;
#[cfg(not(feature = "blank-baseline"))]
pub mod document;
#[cfg(not(feature = "blank-baseline"))]
mod editing;
#[cfg(not(feature = "blank-baseline"))]
mod files;
#[cfg(not(feature = "blank-baseline"))]
mod fonts;
#[cfg(not(feature = "blank-baseline"))]
mod git_workbench;
#[cfg(not(feature = "blank-baseline"))]
mod library;
#[cfg(not(feature = "blank-baseline"))]
mod open_requests;
#[cfg(not(feature = "blank-baseline"))]
mod power;
#[cfg(not(feature = "blank-baseline"))]
mod preferences;
#[cfg(not(feature = "blank-baseline"))]
mod private_store;
#[cfg(not(feature = "blank-baseline"))]
mod reader_assets;
#[cfg(not(feature = "blank-baseline"))]
mod records;
#[cfg(not(feature = "blank-baseline"))]
mod review;
#[cfg(not(feature = "blank-baseline"))]
mod scoped_file;
#[cfg(not(feature = "blank-baseline"))]
pub mod settings;

#[cfg(not(feature = "blank-baseline"))]
pub use app::run;

#[cfg(feature = "blank-baseline")]
pub fn run() {
    let started = std::time::Instant::now();
    tauri::Builder::default()
        .on_page_load(move |_, payload| {
            if std::env::var_os("MARKTUB_PROFILE_STARTUP").as_deref()
                == Some(std::ffi::OsStr::new("1"))
                && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
            {
                eprintln!("MARKTUB_PAGE_LOADED_US={}", started.elapsed().as_micros());
            }
        })
        .run(tauri::generate_context!())
        .expect("Blank baseline could not start");
}

#[cfg(feature = "e2e")]
mod test_support;

/// Private native child entry; it never initializes a WebView or an app command.
#[cfg(not(feature = "blank-baseline"))]
pub fn job_supervisor_entry() -> Option<i32> {
    power::process::supervisor::entry()
}
