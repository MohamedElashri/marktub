use crate::files::{Descriptor, Documents, FileError, SourceRange};
use crate::{
    document::journal::Batch,
    editing::{Applied, Comparison, EditDescriptor, EditState, Editing, Recovery, Saved},
    records::{Position, Records, SettingsResult},
};
use serde::Serialize;
use std::sync::Mutex;
use tauri::{Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

#[derive(Serialize)]
struct OpenResult {
    document: Option<Descriptor>,
    error: Option<String>,
}
#[tauri::command]
async fn take_os_open(app: tauri::AppHandle) -> Result<Vec<OpenResult>, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let (paths, rejected) = app.state::<Mutex<crate::open_requests::OpenRequests>>()
            .lock().map_err(|_| FileError::new("open", "Native open queue unavailable."))?.take();
        let mut protected = editing_path_guard(&app)?;
        if let Ok(path) = app.path().app_data_dir() { protected.push(path); }
        if let Ok(path) = app.path().resource_dir() { protected.push(path); }
        let mut results = Vec::new();
        if rejected { results.push(OpenResult { document: None, error: Some("Some native open requests expired or exceeded the 16-file/local Markdown or text policy. Open those files explicitly.".into()) }); }
        for path in paths {
            let result = (|| {
                let canonical = std::fs::canonicalize(&path)?;
                if protected.iter().any(|p|canonical.starts_with(p)) {
                    return Err(FileError::new("open", "App-owned files cannot be opened through OS handoff. Use recovery or the bundled example."));
                }
                let docs = app.state::<Documents>();
                    let document = grant(&app, &docs, &path)?;
                    // Check the final captured grant, not only the earlier path hint.
                    let scoped = docs.scoped(&document.id)?;
                    if protected.iter().any(|p| scoped.canonical_path().starts_with(p)) {
                        docs.release(&document.id)?;
                        return Err(FileError::new("open", "The native open target changed into an app-owned location."));
                    }
                    Ok(document)
            })();
            results.push(match result {
                Ok(document) => OpenResult { document: Some(document), error: None },
                Err(error) => OpenResult { document: None, error: Some(error.message) },
            });
        }
        Ok(results)
    }).await.map_err(|_| FileError::new("open", "Native file open interrupted."))?
}

fn queue_os_arguments(app: &tauri::AppHandle, args: Vec<String>, cwd: &str) {
    if let Ok(mut queue) = app
        .state::<Mutex<crate::open_requests::OpenRequests>>()
        .lock()
    {
        queue.arguments(args, std::path::Path::new(cwd));
    }
}

fn os_arguments(app: &tauri::AppHandle, args: Vec<String>, cwd: &str) {
    queue_os_arguments(app, args, cwd);
    let _ = app.emit_to("main", "os-open-available", ());
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[derive(Clone, Serialize)]
struct Invalidation {
    id: String,
}

fn grant(
    app: &tauri::AppHandle,
    docs: &Documents,
    path: &std::path::Path,
) -> Result<Descriptor, FileError> {
    let events = app.clone();
    docs.grant_with_policy(
        path,
        !path.starts_with(app.path().resource_dir().unwrap_or_default()),
        move |id| {
            let _ = events.emit_to("main", "document-invalidated", Invalidation { id });
        },
    )
}

#[tauri::command]
async fn choose_document(app: tauri::AppHandle) -> Result<Option<Descriptor>, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(feature = "e2e")]
        if let Some(path) = app.state::<crate::test_support::Fixtures>().take_open() {
            return grant(&app, &app.state::<Documents>(), &path).map(Some);
        }
        let selected = app
            .dialog()
            .file()
            .add_filter("Markdown or text", &["md", "markdown", "txt"])
            .blocking_pick_file();
        selected
            .map(|path| {
                let path = path
                    .into_path()
                    .map_err(|_| FileError::new("path", "Only local files are supported."))?;
                grant(&app, &app.state::<Documents>(), &path)
            })
            .transpose()
    })
    .await
    .map_err(|_| FileError::new("task", "File selection was interrupted."))?
}

#[tauri::command]
async fn open_example(app: tauri::AppHandle) -> Result<Descriptor, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = app
            .path()
            .resource_dir()
            .map_err(|_| FileError::new("resource", "Example unavailable."))?
            .join("fixtures/specimen.md");
        grant(&app, &app.state::<Documents>(), &path)
    })
    .await
    .map_err(|_| FileError::new("task", "Opening the example was interrupted."))?
}

#[tauri::command]
async fn read_range(
    app: tauri::AppHandle,
    id: String,
    start: u64,
    limit: usize,
) -> Result<SourceRange, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Documents>().read_range(&id, start, limit)
    })
    .await
    .map_err(|_| FileError::new("task", "Reading was interrupted."))?
}

#[tauri::command]
fn release_document(docs: State<'_, Documents>, id: String) -> Result<(), FileError> {
    docs.release(&id)
}

#[tauri::command]
async fn document_status(app: tauri::AppHandle, id: String) -> Result<&'static str, FileError> {
    tauri::async_runtime::spawn_blocking(move || app.state::<Documents>().status(&id))
        .await
        .map_err(|_| FileError::new("task", "File validation was interrupted."))?
}

#[tauri::command]
fn execution_status(app: tauri::AppHandle) -> bool {
    powers(&app)
        .lock()
        .map(|p| {
            let s = p.state();
            s.shell || s.sqlite || s.tex
        })
        .unwrap_or(false)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeInfo {
    version: &'static str,
    platform: &'static str,
    writable: bool,
}

#[tauri::command]
fn runtime_info() -> RuntimeInfo {
    RuntimeInfo {
        version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        writable: true,
    }
}

struct EditHost(Mutex<Result<Editing, FileError>>);
async fn editing<T: Send + 'static>(
    app: tauri::AppHandle,
    action: impl FnOnce(&mut Editing, &Documents) -> Result<T, FileError> + Send + 'static,
) -> Result<T, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<EditHost>();
        let mut guard = host
            .0
            .lock()
            .map_err(|_| FileError::new("state", "Editing service unavailable."))?;
        let service = guard.as_mut().map_err(|e| e.clone())?;
        action(service, &app.state::<Documents>())
    })
    .await
    .map_err(|_| {
        FileError::new(
            "task",
            "Editing operation was interrupted; recovery records were kept.",
        )
    })?
}
#[tauri::command]
async fn begin_edit(
    app: tauri::AppHandle,
    id: String,
    start: Option<usize>,
) -> Result<EditDescriptor, FileError> {
    editing(app, move |e, d| e.begin_at(d, &id, start)).await
}
#[tauri::command]
async fn capture_edit_read(
    app: tauri::AppHandle,
    id: String,
    generation: String,
    revision: u64,
) -> Result<String, FileError> {
    editing(app, move |e, _| e.capture_read(&id, &generation, revision)).await
}
#[tauri::command]
async fn read_edit_snapshot(
    app: tauri::AppHandle,
    id: String,
    generation: String,
    token: String,
    start: usize,
) -> Result<crate::files::SourceRange, FileError> {
    editing(app, move |e, _| {
        e.read_snapshot(&id, &generation, &token, start)
    })
    .await
}
#[tauri::command]
async fn change_edit_window(
    app: tauri::AppHandle,
    id: String,
    generation: String,
    revision: u64,
    start: usize,
    from_byte: Option<bool>,
) -> Result<EditDescriptor, FileError> {
    editing(app, move |e, _| {
        let start = if from_byte.unwrap_or(false) {
            e.byte_anchor(&id, &generation, start)?
        } else {
            start
        };
        e.change_window(&id, &generation, revision, start)
    })
    .await
}
#[tauri::command]
async fn apply_edits(
    app: tauri::AppHandle,
    id: String,
    generation: String,
    batches: Vec<Batch>,
    retained: Vec<String>,
) -> Result<Applied, FileError> {
    editing(app, move |e, _| {
        e.apply(&id, &generation, batches, retained)
    })
    .await
}
#[tauri::command]
async fn flush_edits(
    app: tauri::AppHandle,
    id: String,
    generation: String,
) -> Result<EditState, FileError> {
    editing(app, move |e, _| e.flush(&id, &generation)).await
}
#[tauri::command]
async fn save_document(
    app: tauri::AppHandle,
    id: String,
    generation: String,
    revision: u64,
) -> Result<Saved, FileError> {
    editing(app, move |e, _| {
        #[cfg(feature = "e2e")]
        std::thread::sleep(std::time::Duration::from_millis(500));
        e.save(&id, &generation, revision, None, vec![])
    })
    .await
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveAsResult {
    saved: Saved,
    document: Option<Descriptor>,
    refresh_warning: Option<String>,
}
#[tauri::command]
async fn save_as_document(
    app: tauri::AppHandle,
    id: String,
    generation: String,
    revision: u64,
    pending: Vec<Batch>,
) -> Result<Option<SaveAsResult>, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        {
            let host = app.state::<EditHost>();
            let guard = host
                .0
                .lock()
                .map_err(|_| FileError::new("state", "Editing service unavailable."))?;
            guard
                .as_ref()
                .map_err(|e| e.clone())?
                .authorize(&id, &generation)?;
        }
        #[cfg(feature = "e2e")]
        let injected = app.state::<crate::test_support::Fixtures>().take_save();
        #[cfg(not(feature = "e2e"))]
        let injected: Option<std::path::PathBuf> = None;
        let path = if let Some(path) = injected {
            Some(path)
        } else {
            app.dialog()
                .file()
                .set_file_name("Untitled.md")
                .blocking_save_file()
                .map(|p| {
                    p.into_path()
                        .map_err(|_| FileError::new("path", "Select a local destination."))
                })
                .transpose()?
        };
        let Some(path) = path else { return Ok(None) };
        let saved = {
            let host = app.state::<EditHost>();
            let mut guard = host
                .0
                .lock()
                .map_err(|_| FileError::new("state", "Editing service unavailable."))?;
            guard.as_mut().map_err(|e| e.clone())?.save(
                &id,
                &generation,
                revision,
                Some(&path),
                pending,
            )?
        };
        let (document,refresh_warning)=match grant(&app,&app.state::<Documents>(),&path){Ok(doc)=>(Some(doc),None),Err(_)=>(None,Some("The copy was saved, but its reader snapshot could not refresh. Reopen it to refresh navigation and per-file settings.".into()))};
        Ok(Some(SaveAsResult { saved, document, refresh_warning }))
    })
    .await
    .map_err(|_| FileError::new("task", "Save As was interrupted; recovery was kept."))?
}
#[tauri::command]
async fn editing_status(
    app: tauri::AppHandle,
    id: String,
    generation: String,
) -> Result<&'static str, FileError> {
    editing(app, move |e, _| e.status(&id, &generation)).await
}
#[tauri::command]
async fn compare_document(
    app: tauri::AppHandle,
    id: String,
    generation: String,
) -> Result<Comparison, FileError> {
    editing(app, move |e, _| e.compare(&id, &generation)).await
}
#[tauri::command]
async fn reopen_document(app: tauri::AppHandle, id: String) -> Result<Descriptor, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let docs = app.state::<Documents>();
        let target = docs.scoped(&id)?;
        target.validate_selection()?;
        grant(&app, &docs, target.canonical_path())
    })
    .await
    .map_err(|_| FileError::new("task", "Reload was interrupted."))?
}
#[tauri::command]
async fn release_editor(
    app: tauri::AppHandle,
    id: String,
    generation: String,
    discard: bool,
) -> Result<(), FileError> {
    editing(app, move |e, _| e.release(&id, &generation, discard)).await
}
#[tauri::command]
async fn list_recoveries(
    app: tauri::AppHandle,
    include_saved: Option<bool>,
) -> Result<Vec<Recovery>, FileError> {
    editing(app, move |e, _| {
        e.list_with_saved(include_saved.unwrap_or(false))
    })
    .await
}
#[tauri::command]
async fn restore_recovery(app: tauri::AppHandle, id: String) -> Result<EditDescriptor, FileError> {
    editing(app, move |e, _| e.restore(&id)).await
}
#[tauri::command]
async fn discard_recovery(app: tauri::AppHandle, id: String) -> Result<(), FileError> {
    editing(app, move |e, _| e.discard(&id)).await
}
#[tauri::command]
async fn get_settings(
    app: tauri::AppHandle,
    id: Option<String>,
    scope: String,
) -> Result<SettingsResult, FileError> {
    let workspace = id
        .as_ref()
        .and_then(|id| app.state::<Documents>().scoped(id).ok())
        .and_then(|s| library(&app).lock().ok()?.workspace(s.canonical_path()));
    editing(app, move |e, d| {
        if let Some(id) = &id {
            e.workspace(id, workspace);
        }
        e.settings(d, id.as_deref(), &scope)
    })
    .await
}
#[tauri::command]
async fn update_settings(
    app: tauri::AppHandle,
    id: Option<String>,
    scope: String,
    text: String,
) -> Result<SettingsResult, FileError> {
    let workspace = id
        .as_ref()
        .and_then(|id| app.state::<Documents>().scoped(id).ok())
        .and_then(|s| library(&app).lock().ok()?.workspace(s.canonical_path()));
    let result = editing(app.clone(), move |e, d| {
        if let Some(id) = &id {
            e.workspace(id, workspace);
        }
        e.update_settings(d, id.as_deref(), &scope, &text)
    })
    .await?;
    if result.diagnostics.is_empty() {
        let _ = app.emit_to("main", "preferences-changed", ());
    }
    Ok(result)
}
#[tauri::command]
async fn import_settings(app: tauri::AppHandle) -> Result<Option<String>, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let selected = app
            .dialog()
            .file()
            .add_filter("Settings", &["conf", "txt"])
            .blocking_pick_file();
        selected
            .map(|p| {
                let path = p
                    .into_path()
                    .map_err(|_| FileError::new("path", "Select a local settings file."))?;
                let canonical = std::fs::canonicalize(&path)?;
                let scoped = crate::scoped_file::ScopedFile::selected(&canonical)?
                    .authorize_selection(&path);
                let (mut file, _, length) = scoped.snapshot_bounded(65536)?;
                if length > 65536 {
                    return Err(FileError::new("capacity", "Settings exceed 64 KiB."));
                }
                use std::io::Read;
                let mut text = String::new();
                file.read_to_string(&mut text)?;
                Ok(text)
            })
            .transpose()
    })
    .await
    .map_err(|_| FileError::new("task", "Settings import was interrupted."))?
}
#[tauri::command]
async fn export_settings(app: tauri::AppHandle, text: String) -> Result<bool, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = app
            .dialog()
            .file()
            .set_file_name("marktub.conf")
            .blocking_save_file()
        else {
            return Ok(false);
        };
        let path = path
            .into_path()
            .map_err(|_| FileError::new("path", "Select a local destination."))?;
        if let Ok(root) = app.path().app_data_dir()
            && let Some(parent) = path.parent()
            && std::fs::canonicalize(parent)?.starts_with(root)
        {
            return Err(FileError::new(
                "private-storage",
                "Choose an export destination outside app-private storage.",
            ));
        }
        Records::export(&path, &text)?;
        Ok(true)
    })
    .await
    .map_err(|_| FileError::new("task", "Settings export was interrupted."))?
}
#[tauri::command]
async fn reset_settings(app: tauri::AppHandle) -> Result<(), FileError> {
    editing(app.clone(), |e, _| e.reset_settings()).await?;
    let _ = app.emit_to("main", "preferences-changed", ());
    Ok(())
}
#[tauri::command]
async fn document_position(app: tauri::AppHandle, id: String) -> Result<Position, FileError> {
    editing(app, move |e, d| e.position(d, &id)).await
}
#[tauri::command]
async fn remember_position(
    app: tauri::AppHandle,
    id: String,
    position: Position,
) -> Result<(), FileError> {
    editing(app, move |e, d| e.remember(d, &id, position)).await
}
#[tauri::command]
async fn storage_warning(app: tauri::AppHandle) -> Result<Option<String>, FileError> {
    editing(app, |e, _| Ok(e.warning())).await
}
#[tauri::command]
async fn close_application(app: tauri::AppHandle) -> Result<(), FileError> {
    if powers(&app)
        .lock()
        .map_err(|_| power::err("Local jobs unavailable."))?
        .jobs
        .values()
        .any(|j| j.result.lock().unwrap().status == "running")
    {
        return Err(power::err(
            "Cancel the local job and wait for its result before quitting.",
        ));
    }

    if library(&app)
        .lock()
        .map_err(|_| FileError::new("state", "Library unavailable."))?
        .is_running()
    {
        return Err(FileError::new(
            "busy",
            "Cancel the library job and wait for its result before quitting.",
        ));
    }
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let host = handle.state::<EditHost>();
        let mut guard = host.0.lock().map_err(|_| {
            FileError::new(
                "state",
                "Editing service unavailable; copy unsaved text before closing.",
            )
        })?;
        if let Ok(service) = guard.as_mut() {
            service.flush_all()?;
        }
        Ok::<_, FileError>(())
    })
    .await
    .map_err(|_| FileError::new("task", "Close was interrupted."))??;
    app.exit(0);
    Ok(())
}

macro_rules! commands {($($extra:ident),*)=>{tauri::generate_handler![take_os_open,power_state,power_enable,power_choose_tool,power_trust,power_choose_database,power_capture,power_release,power_image,power_diagram,power_prepare,power_run,power_poll,power_cancel,power_clear,power_save_pdf,power_open_pdf,review_begin,review_sync,review_displayed,review_end,review_update,review_compare,review_export,review_save,review_list,review_delete,review_archive_export,review_archive_save,git_badges,git_history_files,git_choose,git_refresh,git_revoke,git_compare,git_page,git_mutate,git_open,git_backups,git_export_backup,git_forget_backup,capture_edit_read,read_edit_snapshot,import_author_image,change_edit_window,preference_catalogue,list_fonts,preview_settings,library_location,library_session,library_remember_document,library_accept_drop,library_state,library_choose_root,library_page,library_open,library_preview,library_scan,library_job,library_prepare,library_commit,library_configure,library_pin,library_remove_root,open_document_link,choose_image,export_html,choose_document,open_example,read_range,release_document,document_status,runtime_info,execution_status,
    begin_edit,apply_edits,flush_edits,save_document,save_as_document,editing_status,compare_document,reopen_document,release_editor,
    list_recoveries,restore_recovery,discard_recovery,get_settings,update_settings,import_settings,export_settings,document_position,remember_position,storage_warning,close_application,reset_settings,$($extra),*]};}

// Explicit local diagnostics, never a frontend command/evaluator or telemetry.
fn startup_stage(start: Option<std::time::Instant>, name: &str) {
    if let Some(start) = start {
        eprintln!(
            "MARKTUB_STARTUP_STAGE={name}:{}",
            start.elapsed().as_micros()
        );
    }
}

pub fn run() {
    let trace = (std::env::var_os("MARKTUB_PROFILE_STARTUP").as_deref()
        == Some(std::ffi::OsStr::new("1")))
    .then(std::time::Instant::now);
    startup_stage(trace, "entry");
    crate::git_workbench::initialize().expect("Local Git configuration isolation failed");
    startup_stage(trace, "git-isolation");
    let builder = tauri::Builder::default()
        .manage(Mutex::new(crate::open_requests::OpenRequests::default()))
        .plugin(tauri_plugin_single_instance::init(|app, args, cwd| {
            os_arguments(app, args, &cwd)
        }));
    #[cfg(feature = "e2e")]
    let builder = builder
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init())
        .manage(crate::test_support::Fixtures::default());
    let started = std::time::Instant::now();
    let builder = builder
        .on_page_load(move |_, payload| {
            if trace.is_some() && matches!(payload.event(), tauri::webview::PageLoadEvent::Finished)
            {
                startup_stage(trace, "page-loaded");
                eprintln!("MARKTUB_PAGE_LOADED_US={}", started.elapsed().as_micros());
            }
        })
        .manage(Documents::default())
        .plugin(tauri_plugin_dialog::init())
        .setup(move |app| {
            startup_stage(trace, "setup-begin");
            let path = app
                .path()
                .app_data_dir()
                .map(|p| p.join("private-v1"))
                .map_err(|_| {
                    FileError::new("private-storage", "App-private storage is unavailable.")
                });
            #[cfg(feature = "e2e")]
            let path =
                path.map(|p| p.with_file_name(format!("e2e-private-{}", std::process::id())));
            let service = path.and_then(|p| Editing::open(&p)).map(|mut service| {
                if let Ok(resources) = app.path().resource_dir() {
                    service.protect(&resources);
                }
                service
            });
            startup_stage(trace, "storage-open");
            let (store, protected) = service
                .as_ref()
                .map(|e| e.library_config())
                .unwrap_or_else(|_| (std::path::PathBuf::new(), vec![]));
            app.manage(LibraryHost(std::sync::Arc::new(Mutex::new(
                crate::library::Library::new(store.clone(), protected.clone()),
            ))));
            app.manage(ReviewHost(Mutex::new(crate::review::Reviews::new(
                store.clone(),
            ))));
            let mut power_protected = protected.clone();
            if let Ok(path) = app.path().app_data_dir() {
                power_protected.push(path);
            }
            app.manage(PowerHost(std::sync::Arc::new(Mutex::new(
                power::Powers::new(store.clone(), power_protected),
            ))));
            app.manage(GitHost(Mutex::new(crate::git_workbench::Workbench::new(
                store, protected,
            ))));
            app.manage(EditHost(Mutex::new(service)));
            // The normal Tauri window owns initial visibility/focus. Only an
            // actual second-instance handoff needs to raise an existing window.
            queue_os_arguments(
                app.handle(),
                std::env::args_os()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect(),
                &std::env::current_dir()
                    .unwrap_or_default()
                    .to_string_lossy(),
            );
            startup_stage(trace, "setup-complete");
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop { paths, .. }) = event
                && let Ok(mut l) = library(window.app_handle()).lock()
            {
                let offers = l.offer(paths);
                let _ = window.emit("library-drop", offers);
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.emit("close-requested", ());
            }
        });
    #[cfg(feature = "e2e")]
    let builder = builder.invoke_handler(commands![e2e_fixture]);
    #[cfg(not(feature = "e2e"))]
    let builder = builder.invoke_handler(commands![]);
    builder
        .build(tauri::generate_context!())
        .expect("Marktub could not start")
        .run(|app, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { ref urls } = event {
                if let Ok(mut queue) = app
                    .state::<Mutex<crate::open_requests::OpenRequests>>()
                    .lock()
                {
                    queue.push(urls.iter().filter_map(|url| url.to_file_path().ok()));
                }
                let _ = app.emit_to("main", "os-open-available", ());
            }
            if let tauri::RunEvent::ExitRequested {
                code: None, api, ..
            } = event
            {
                api.prevent_exit();
                let _ = app.emit_to("main", "close-requested", ());
            }
        });
}

#[cfg(feature = "e2e")]
#[tauri::command]
async fn e2e_fixture(
    app: tauri::AppHandle,
    action: String,
    text: Option<String>,
) -> Result<crate::test_support::FixtureResult, FileError> {
    if action == "library-drop" || action == "author-drop" {
        let path = app
            .state::<crate::test_support::Fixtures>()
            .root()
            .ok_or_else(|| FileError::new("test", "Seed a fixture first."))?;
        let path = if action == "author-drop" {
            path.join("pixel.png")
        } else {
            path
        };
        let offers = library(&app).lock().unwrap().offer(&[path]);
        let _ = app.emit_to("main", "library-drop", offers);
        return app
            .state::<crate::test_support::Fixtures>()
            .action("read", None);
    }
    if action == "request-close" {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| FileError::new("test", "Main window unavailable."))?;
        std::thread::spawn(move || window.close())
            .join()
            .map_err(|_| FileError::new("test", "Close thread failed."))?
            .map_err(|_| FileError::new("test", "Close request failed."))?;
        return app
            .state::<crate::test_support::Fixtures>()
            .action("read", None);
    }
    app.state::<crate::test_support::Fixtures>()
        .action(&action, text)
}

#[tauri::command]
async fn open_document_link(
    app: tauri::AppHandle,
    id: String,
    link: String,
) -> Result<Option<Descriptor>, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Documents>().byte_length(&id)?;
        if link.len() > 2048 || link.chars().any(char::is_control) {
            return Err(FileError::new("link", "Invalid document link."));
        }
        let scoped = app.state::<Documents>().scoped(&id)?;
        let resolved = library(&app)
            .lock()
            .map_err(|_| FileError::new("state", "Library unavailable."))?
            .resolve_link(scoped.canonical_path(), &link)?;
        if let Some(scoped) = resolved {
            let events = app.clone();
            return app
                .state::<Documents>()
                .grant_scoped(scoped, true, move |id| {
                    let _ = events.emit_to("main", "document-invalidated", Invalidation { id });
                })
                .map(Some);
        }
        #[cfg(feature = "e2e")]
        if let Some(path) = app.state::<crate::test_support::Fixtures>().take_open() {
            return grant(&app, &app.state::<Documents>(), &path).map(Some);
        }
        let selected = app
            .dialog()
            .file()
            .set_title(format!(
                "Authorize linked document: {}",
                link.chars().take(120).collect::<String>()
            ))
            .add_filter("Markdown or text", &["md", "markdown", "txt"])
            .blocking_pick_file();
        selected
            .map(|p| {
                let path = p
                    .into_path()
                    .map_err(|_| FileError::new("path", "Select a local document."))?;
                grant(&app, &app.state::<Documents>(), &path)
            })
            .transpose()
    })
    .await
    .map_err(|_| FileError::new("task", "Link selection interrupted."))?
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuthorImage {
    path: String,
    data_url: String,
    width: u32,
    height: u32,
}
#[tauri::command]
async fn import_author_image(
    app: tauri::AppHandle,
    id: String,
    copy: bool,
    offer: Option<String>,
    bytes: Option<Vec<u8>>,
) -> Result<Option<AuthorImage>, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let doc = app.state::<Documents>().scoped(&id)?;
        let parent = doc
            .canonical_path()
            .parent()
            .ok_or_else(|| FileError::new("path", "Document has no parent."))?;
        let (source, image) = if let Some(bytes) = bytes {
            if !copy || offer.is_some() || bytes.len() > 2 * 1024 * 1024 {
                return Err(FileError::new(
                    "image",
                    "Clipboard image exceeds its budget.",
                ));
            }
            (None, crate::reader_assets::image_bytes(&bytes)?)
        } else {
            let selected = if let Some(offer) = offer {
                Some(
                    library(&app)
                        .lock()
                        .map_err(|_| FileError::new("state", "Library unavailable."))?
                        .accept_drop(&offer)?,
                )
            } else {
                #[cfg(feature = "e2e")]
                let fixture = app.state::<crate::test_support::Fixtures>().take_image();
                #[cfg(not(feature = "e2e"))]
                let fixture: Option<std::path::PathBuf> = None;
                if fixture.is_some() {
                    fixture
                } else {
                    app.dialog()
                        .file()
                        .add_filter("Raster image", &["png", "jpg", "jpeg"])
                        .blocking_pick_file()
                        .map(|p| {
                            p.into_path()
                                .map_err(|_| FileError::new("path", "Select a local image."))
                        })
                        .transpose()?
                }
            };
            let Some(path) = selected else {
                return Ok(None);
            };
            let image = crate::reader_assets::image(&path)?;
            (Some(path), image)
        };
        let path = if copy {
            let assets = parent.join("assets");
            let proposed = if assets.is_dir() {
                assets
            } else {
                parent.to_owned()
            };
            let name = source
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|p| p.to_str())
                .unwrap_or("pasted.png");
            #[cfg(feature = "e2e")]
            let fixture = app
                .state::<crate::test_support::Fixtures>()
                .take_save()
                .map(|p| p.with_file_name("asset.png"));
            #[cfg(not(feature = "e2e"))]
            let fixture: Option<std::path::PathBuf> = None;
            let path = if let Some(path) = fixture {
                path
            } else {
                let Some(dest) = app
                    .dialog()
                    .file()
                    .set_title("Copy image to a new file")
                    .set_directory(proposed)
                    .set_file_name(name)
                    .blocking_save_file()
                else {
                    return Ok(None);
                };
                dest.into_path()
                    .map_err(|_| FileError::new("path", "Choose a local destination."))?
            };
            let target = crate::scoped_file::ScopedFile::destination(&path)?;
            for protected in [
                app.path().app_data_dir().ok(),
                app.path().resource_dir().ok(),
            ]
            .into_iter()
            .flatten()
            {
                let protected = std::fs::canonicalize(&protected).unwrap_or(protected);
                if target.canonical_path().starts_with(protected) {
                    return Err(FileError::new(
                        "private-storage",
                        "Choose an asset outside app-owned storage.",
                    ));
                }
            }
            use base64::Engine;
            let data = base64::engine::general_purpose::STANDARD
                .decode(
                    image
                        .data_url
                        .split_once(',')
                        .ok_or_else(|| FileError::new("image", "Invalid raster encoding."))?
                        .1,
                )
                .map_err(|_| FileError::new("image", "Invalid raster encoding."))?;
            crate::atomic_save::write_asset(&target, &data)?;
            target.canonical_path().to_owned()
        } else {
            std::fs::canonicalize(
                source.ok_or_else(|| FileError::new("image", "Choose an image."))?,
            )?
        };
        let src: Vec<_> = path.components().collect();
        let base: Vec<_> = parent.components().collect();
        let common = src.iter().zip(&base).take_while(|(a, b)| a == b).count();
        let mut relative = std::path::PathBuf::new();
        for _ in common..base.len() {
            relative.push("..");
        }
        for part in &src[common..] {
            relative.push(part.as_os_str());
        }
        let relative = relative
            .to_str()
            .ok_or_else(|| {
                FileError::new(
                    "encoding",
                    "Image path is not UTF-8; no reference was inserted.",
                )
            })?
            .to_owned();
        Ok(Some(AuthorImage {
            path: relative,
            data_url: image.data_url,
            width: image.width,
            height: image.height,
        }))
    })
    .await
    .map_err(|_| {
        FileError::new(
            "task",
            "Image import interrupted; copied files are retained.",
        )
    })?
}
#[tauri::command]
async fn choose_image(
    app: tauri::AppHandle,
    id: String,
    link: String,
) -> Result<Option<crate::reader_assets::Image>, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Documents>().byte_length(&id)?;
        if link.len() > 2048 {
            return Err(FileError::new("image", "Image hint is too long."));
        }
        let scoped = app.state::<Documents>().scoped(&id)?;
        if let Some(scoped) = library(&app)
            .lock()
            .map_err(|_| FileError::new("state", "Library unavailable."))?
            .resolve_link(scoped.canonical_path(), &link)?
        {
            return crate::reader_assets::image_scoped(scoped).map(Some);
        }
        #[cfg(feature = "e2e")]
        if let Some(path) = app.state::<crate::test_support::Fixtures>().take_image() {
            return crate::reader_assets::image(&path).map(Some);
        }
        let selected = app
            .dialog()
            .file()
            .set_title(format!(
                "Authorize image: {}",
                link.chars()
                    .filter(|c| !c.is_control())
                    .take(120)
                    .collect::<String>()
            ))
            .add_filter("Local raster image", &["png", "jpg", "jpeg"])
            .blocking_pick_file();
        selected
            .map(|p| {
                let path = p
                    .into_path()
                    .map_err(|_| FileError::new("path", "Select a local image."))?;
                crate::reader_assets::image(&path)
            })
            .transpose()
    })
    .await
    .map_err(|_| FileError::new("task", "Image authorization interrupted."))?
}
#[tauri::command]
async fn export_html(app: tauri::AppHandle, content: String) -> Result<bool, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        if content.len() > 64 * 1024 * 1024 || !content.starts_with("<!doctype html>") {
            return Err(FileError::new(
                "export",
                "Invalid or oversized standalone HTML.",
            ));
        }
        #[cfg(feature = "e2e")]
        let injected = app.state::<crate::test_support::Fixtures>().take_export();
        #[cfg(not(feature = "e2e"))]
        let injected: Option<std::path::PathBuf> = None;
        let path = if let Some(path) = injected {
            path
        } else {
            let Some(path) = app
                .dialog()
                .file()
                .set_title("Export standalone HTML to a new file")
                .set_file_name("document.html")
                .add_filter("HTML", &["html"])
                .blocking_save_file()
            else {
                return Ok(false);
            };
            path.into_path()
                .map_err(|_| FileError::new("path", "Select a local export destination."))?
        };
        let protected = [
            app.path().app_data_dir().ok(),
            app.path().resource_dir().ok(),
        ]
        .into_iter()
        .flatten()
        .filter_map(|p| std::fs::canonicalize(p).ok())
        .collect::<Vec<_>>();
        crate::reader_assets::export(&path, &content, &protected)?;
        Ok(true)
    })
    .await
    .map_err(|_| FileError::new("task", "HTML export interrupted."))?
}

struct LibraryHost(std::sync::Arc<Mutex<crate::library::Library>>);
fn library(app: &tauri::AppHandle) -> std::sync::Arc<Mutex<crate::library::Library>> {
    app.state::<LibraryHost>().0.clone()
}
async fn library_task<T: Send + 'static>(
    app: tauri::AppHandle,
    action: impl FnOnce(&mut crate::library::Library) -> Result<T, FileError> + Send + 'static,
) -> Result<T, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let shared = library(&app);
        let mut guard = shared
            .lock()
            .map_err(|_| FileError::new("state", "Library unavailable."))?;
        action(&mut guard)
    })
    .await
    .map_err(|_| FileError::new("task", "Library operation interrupted."))?
}
#[tauri::command]
async fn library_state(app: tauri::AppHandle) -> Result<crate::library::State, FileError> {
    library_task(app, |l| Ok(l.state())).await
}
#[tauri::command]
async fn library_choose_root(
    app: tauri::AppHandle,
) -> Result<Option<crate::library::Folder>, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(feature = "e2e")]
        if let Some(path) = app.state::<crate::test_support::Fixtures>().root() {
            let selected = library(&app)
                .lock()
                .map_err(|_| FileError::new("state", "Library unavailable."))?
                .select(&path)
                .map(Some)?;
            let _ = app.emit_to("main", "preferences-changed", ());
            return Ok(selected);
        }
        let Some(path) = app
            .dialog()
            .file()
            .set_title("Authorize library folder")
            .blocking_pick_folder()
        else {
            return Ok(None);
        };
        let path = path
            .into_path()
            .map_err(|_| FileError::new("path", "Select a local folder."))?;
        let selected = library(&app)
            .lock()
            .map_err(|_| FileError::new("state", "Library unavailable."))?
            .select(&path)
            .map(Some)?;
        let _ = app.emit_to("main", "preferences-changed", ());
        Ok(selected)
    })
    .await
    .map_err(|_| FileError::new("task", "Folder selection interrupted."))?
}
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Explicit typed IPC fields, with no path argument.
async fn library_page(
    app: tauri::AppHandle,
    id: String,
    offset: usize,
    sort: String,
    hidden: bool,
    filter: String,
    refresh: bool,
    ignored: bool,
) -> Result<crate::library::Page, FileError> {
    library_task(app, move |l| {
        l.page(&id, offset, &sort, hidden, &filter, refresh, ignored)
    })
    .await
}
#[tauri::command]
async fn library_open(
    app: tauri::AppHandle,
    id: String,
    recent: bool,
) -> Result<Descriptor, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = library(&app);
        let mut l = host
            .lock()
            .map_err(|_| FileError::new("state", "Library unavailable."))?;
        let id = if recent { l.resolve_hint(&id)? } else { id };
        let scoped = l.scoped(&id)?;
        l.remember(&id)?;
        drop(l);
        let events = app.clone();
        app.state::<Documents>()
            .grant_scoped(scoped, true, move |id| {
                let _ = events.emit_to("main", "document-invalidated", Invalidation { id });
            })
    })
    .await
    .map_err(|_| FileError::new("task", "Opening library document interrupted."))?
}
#[tauri::command]
async fn library_preview(app: tauri::AppHandle, id: String) -> Result<String, FileError> {
    library_task(app, move |l| l.preview(&id)).await
}
#[tauri::command]
fn library_scan(
    app: tauri::AppHandle,
    query: String,
    content: bool,
    hidden: bool,
    ignored: bool,
) -> Result<String, FileError> {
    crate::library::Library::start(library(&app), query, content, hidden, ignored)
}
#[tauri::command]
async fn library_job(
    app: tauri::AppHandle,
    id: String,
    cancel: bool,
) -> Result<crate::library::Progress, FileError> {
    library_task(app, move |l| l.progress(&id, cancel)).await
}
#[tauri::command]
async fn library_prepare(
    app: tauri::AppHandle,
    request: crate::library::Request,
) -> Result<crate::library::Review, FileError> {
    library_task(app, move |l| l.prepare(request)).await
}
#[tauri::command]
async fn library_commit(app: tauri::AppHandle, id: String) -> Result<String, FileError> {
    editing(app.clone(), |e, _| e.library_reserve()).await?;
    let job_id = id.clone();
    let task = library_task(app.clone(), move |l| l.take_plan(&id)).await;
    match task {
        Ok(task) => {
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task.run()));
                let failed = result.is_err();
                let moved = result.unwrap_or_default();
                for (old, new) in moved {
                    let mut notifications = vec![];
                    let documents = app.state::<Documents>().paths().unwrap_or_default();
                    let mut pairs = library(&app)
                        .lock()
                        .map(|mut l|{let pairs=l.relocate(&old,&new);if let Err(error)=l.select_moved(&new){l.warn(format!("Move completed; refresh its destination to restore the selection: {}",error.message));}pairs})
                        .unwrap_or_default();
                    for (previous, path) in documents {
                        if let Ok(suffix) = path.strip_prefix(&old) {
                            let dest = new.join(suffix);
                            pairs.push((path, dest.clone()));
                            let resolved = (|| {
                                let shared = library(&app);
                                let mut l = shared
                                    .lock()
                                    .map_err(|_| FileError::new("state", "Library unavailable."))?;
                                let id = l.resolve_path(&dest)?;
                                l.scoped(&id)
                            })();
                            if let Ok(scoped) = resolved {
                                let events = app.clone();
                                if let Ok(document) =
                                    app.state::<Documents>()
                                        .grant_scoped(scoped, true, move |id| {
                                            let _ = events.emit_to(
                                                "main",
                                                "document-invalidated",
                                                Invalidation { id },
                                            );
                                        })
                                {
                                    notifications.push(Relocated { previous, document });
                                }
                            }
                        }
                    }
                    if let Ok(mut host) = app.state::<EditHost>().0.lock()
                        && let Ok(e) = host.as_mut()
                    {
                        for (from, to) in pairs {
                            if let Err(error) = e.relocate(&from, &to) {
                                if let Ok(mut l) = library(&app).lock() {
                                    l.warn(format!("File move completed, but saved preferences/positions could not be relocated: {}",error.message));
                                }
                                break;
                            }
                        }
                    }
                    for notification in notifications {
                        let _ = app.emit_to("main", "document-relocated", notification);
                    }
                }
                if let Ok(mut host) = app.state::<EditHost>().0.lock()
                    && let Ok(e) = host.as_mut()
                {
                    e.library_release();
                }
                if let Ok(mut l) = library(&app).lock() {
                    l.finish_mutation(failed);
                }
            });
            Ok(job_id)
        }
        Err(error) => {
            editing(app, |e, _| {
                e.library_release();
                Ok(())
            })
            .await?;
            Err(error)
        }
    }
}
#[tauri::command]
async fn library_configure(
    app: tauri::AppHandle,
    sidebar: bool,
    view: String,
) -> Result<(), FileError> {
    library_task(app, move |l| l.configure(sidebar, &view)).await
}
#[tauri::command]
async fn library_pin(app: tauri::AppHandle, id: String) -> Result<(), FileError> {
    library_task(app, move |l| l.pin(&id)).await
}
#[tauri::command]
async fn library_remove_root(app: tauri::AppHandle, id: String) -> Result<(), FileError> {
    library_task(app.clone(), move |l| l.remove_root(&id)).await?;
    let _ = app.emit_to("main", "preferences-changed", ());
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DropAccepted {
    document: Option<Descriptor>,
    folder: Option<crate::library::Folder>,
}
#[tauri::command]
async fn library_accept_drop(
    app: tauri::AppHandle,
    id: String,
    action: String,
) -> Result<DropAccepted, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let shared = library(&app);
        let mut l = shared
            .lock()
            .map_err(|_| FileError::new("state", "Library unavailable."))?;
        let path = l.accept_drop(&id)?;
        if action == "folder" {
            let folder = l.select(&path)?;
            drop(l);
            let _ = app.emit_to("main", "preferences-changed", ());
            return Ok(DropAccepted {
                document: None,
                folder: Some(folder),
            });
        }
        if action != "open" {
            return Err(FileError::new("action", "Choose Open or Add folder."));
        }
        drop(l);
        grant(&app, &app.state::<Documents>(), &path).map(|document| DropAccepted {
            document: Some(document),
            folder: None,
        })
    })
    .await
    .map_err(|_| FileError::new("task", "Drop authorization interrupted."))?
}

#[derive(Clone, Serialize)]
struct Relocated {
    previous: String,
    document: Descriptor,
}
#[tauri::command]
async fn library_remember_document(app: tauri::AppHandle, id: String) -> Result<(), FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = app
            .state::<Documents>()
            .scoped(&id)?
            .canonical_path()
            .to_owned();
        library(&app)
            .lock()
            .map_err(|_| FileError::new("state", "Library unavailable."))?
            .remember_path(&path);
        Ok(())
    })
    .await
    .map_err(|_| FileError::new("task", "Remembering document interrupted."))?
}

#[tauri::command]
async fn library_session(
    app: tauri::AppHandle,
    ids: Vec<String>,
    active: usize,
) -> Result<(), FileError> {
    if ids.len() > 4 {
        return Err(FileError::new("capacity", "Too many tabs."));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let docs = app.state::<Documents>();
        let paths = ids
            .iter()
            .map(|id| docs.scoped(id).map(|s| s.canonical_path().to_owned()))
            .collect::<Result<Vec<_>, _>>()?;
        library(&app)
            .lock()
            .map_err(|_| FileError::new("state", "Library unavailable."))?
            .session(paths, active)
    })
    .await
    .map_err(|_| FileError::new("task", "Saving session interrupted."))?
}

#[tauri::command]
async fn library_location(
    app: tauri::AppHandle,
    id: String,
    display: crate::library::Display,
) -> Result<(), FileError> {
    library_task(app, move |l| l.location(&id, display)).await
}

#[tauri::command]
fn preference_catalogue() -> Vec<crate::preferences::Spec> {
    crate::preferences::catalogue()
}
#[tauri::command]
async fn list_fonts() -> Result<Vec<String>, FileError> {
    tauri::async_runtime::spawn_blocking(crate::fonts::families)
        .await
        .map_err(|_| FileError::new("fonts", "Installed font names could not be listed."))
}
#[tauri::command]
async fn preview_settings(
    app: tauri::AppHandle,
    id: Option<String>,
    scope: String,
    text: String,
) -> Result<SettingsResult, FileError> {
    let workspace = id
        .as_ref()
        .and_then(|id| app.state::<Documents>().scoped(id).ok())
        .and_then(|s| library(&app).lock().ok()?.workspace(s.canonical_path()));
    editing(app, move |e, d| {
        if let Some(id) = &id {
            e.workspace(id, workspace);
        }
        e.preview_settings(d, id.as_deref(), &scope, &text)
    })
    .await
}

struct GitHost(Mutex<crate::git_workbench::Workbench>);
async fn git_task<T: Send + 'static>(
    app: tauri::AppHandle,
    action: impl FnOnce(&mut crate::git_workbench::Workbench) -> Result<T, FileError> + Send + 'static,
) -> Result<T, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<GitHost>();
        let mut git = host
            .0
            .lock()
            .map_err(|_| FileError::new("git-state", "Git workbench unavailable."))?;
        action(&mut git)
    })
    .await
    .map_err(|_| {
        FileError::new(
            "git-task",
            "Git operation interrupted. Refresh before retrying; recovery copies are kept.",
        )
    })?
}
#[tauri::command]
async fn git_choose(
    app: tauri::AppHandle,
) -> Result<Option<crate::git_workbench::State>, FileError> {
    let chooser = app.clone();
    let selected = tauri::async_runtime::spawn_blocking(move || {
        #[cfg(feature = "e2e")]
        if let Some(path) = chooser.state::<crate::test_support::Fixtures>().root() {
            return Ok(Some(path));
        }
        chooser
            .dialog()
            .file()
            .blocking_pick_folder()
            .map(|p| {
                p.into_path()
                    .map_err(|_| FileError::new("path", "Choose a local repository folder."))
            })
            .transpose()
    })
    .await
    .map_err(|_| FileError::new("git-task", "Repository selection interrupted."))??;
    match selected {
        Some(path) => {
            let result = git_task(app.clone(), move |g| g.choose(&path).map(Some)).await;
            if result.is_ok() {
                let _ = app.emit_to("main", "git-updated", ());
            }
            result
        }
        None => Ok(None),
    }
}
#[tauri::command]
async fn git_refresh(
    app: tauri::AppHandle,
    id: String,
    start: usize,
) -> Result<crate::git_workbench::State, FileError> {
    let result = git_task(app.clone(), move |g| g.refresh(&id, start)).await;
    if result.is_ok() {
        let _ = app.emit_to("main", "git-updated", ());
    }
    result
}
#[tauri::command]
async fn git_revoke(app: tauri::AppHandle, id: String) -> Result<(), FileError> {
    let result = git_task(app.clone(), move |g| g.revoke(&id)).await;
    if result.is_ok() {
        let _ = app.emit_to("main", "git-updated", ());
    }
    result
}
#[tauri::command]
async fn git_compare(
    app: tauri::AppHandle,
    id: String,
    file: String,
    before: String,
    after: String,
) -> Result<crate::git_workbench::Diff, FileError> {
    git_task(app, move |g| g.compare(&id, &file, &before, &after)).await
}
#[tauri::command]
async fn git_page(
    app: tauri::AppHandle,
    id: String,
    preview: String,
    start: usize,
    split: bool,
    expanded: bool,
) -> Result<Vec<crate::git_workbench::Row>, FileError> {
    git_task(app, move |g| g.page(&id, &preview, start, split, expanded)).await
}
#[tauri::command]
async fn git_mutate(
    app: tauri::AppHandle,
    id: String,
    preview: String,
    action: String,
    hunks: Vec<String>,
) -> Result<crate::git_workbench::Mutated, FileError> {
    editing(app.clone(), |e, _| e.library_reserve()).await?;
    let result = git_task(app.clone(), move |g| {
        g.mutate(&id, &preview, &action, &hunks)
    })
    .await;
    let release = editing(app, |e, _| {
        e.library_release();
        Ok(())
    })
    .await;
    match (result, release) {
        (Ok(x), Ok(())) => Ok(x),
        (Err(e), _) => Err(e),
        (_, Err(e)) => Err(e),
    }
}
#[tauri::command]
async fn git_open(
    app: tauri::AppHandle,
    id: String,
    file: String,
) -> Result<Descriptor, FileError> {
    let scoped = git_task(app.clone(), move |g| g.scoped(&id, &file)).await?;
    let events = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<Documents>()
            .grant_scoped(scoped, true, move |id| {
                let _ = events.emit_to("main", "document-invalidated", Invalidation { id });
            })
    })
    .await
    .map_err(|_| FileError::new("git-task", "Opening the selected Git file interrupted."))?
}
#[tauri::command]
async fn git_backups(
    app: tauri::AppHandle,
    id: String,
) -> Result<Vec<crate::git_workbench::Backup>, FileError> {
    git_task(app, move |g| g.backups(&id)).await
}
#[tauri::command]
async fn git_forget_backup(
    app: tauri::AppHandle,
    id: String,
    backup: String,
) -> Result<(), FileError> {
    git_task(app, move |g| g.forget_backup(&id, &backup)).await
}
#[tauri::command]
async fn git_export_backup(
    app: tauri::AppHandle,
    id: String,
    backup: String,
) -> Result<bool, FileError> {
    let bytes = git_task(app.clone(), move |g| g.backup_bytes(&id, &backup)).await?;
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(feature = "e2e")]
        let selected = app
            .state::<crate::test_support::Fixtures>()
            .take_save()
            .map(tauri_plugin_dialog::FilePath::Path);
        #[cfg(not(feature = "e2e"))]
        let selected: Option<tauri_plugin_dialog::FilePath> = None;
        let selected = selected.or_else(|| {
            app.dialog()
                .file()
                .set_file_name("git-recovered.txt")
                .blocking_save_file()
        });
        let Some(path) = selected else {
            return Ok(false);
        };
        let path = path
            .into_path()
            .map_err(|_| FileError::new("path", "Choose a new local destination."))?;
        let target = crate::scoped_file::ScopedFile::destination(&path)?;
        let protected = editing_path_guard(&app)?;
        if protected
            .iter()
            .any(|p| target.canonical_path().starts_with(p))
        {
            return Err(FileError::new(
                "private-storage",
                "Choose a destination outside app-owned resources.",
            ));
        }
        let source = crate::document::Provenance::new(&bytes).map_err(|_| {
            FileError::new(
                "git-recovery",
                "Recovery is not supported UTF-8; original bytes are retained.",
            )
        })?;
        crate::atomic_save::save_as(&target, &source.snapshot())?;
        Ok(true)
    })
    .await
    .map_err(|_| {
        FileError::new(
            "git-task",
            "Export interrupted; original recovery bytes are kept.",
        )
    })?
}
fn editing_path_guard(app: &tauri::AppHandle) -> Result<Vec<std::path::PathBuf>, FileError> {
    let host = app.state::<EditHost>();
    let guard = host
        .0
        .lock()
        .map_err(|_| FileError::new("state", "Editing service unavailable."))?;
    Ok(guard.as_ref().map_err(Clone::clone)?.library_config().1)
}

#[tauri::command]
async fn git_history_files(
    app: tauri::AppHandle,
    id: String,
    commit: String,
    start: usize,
) -> Result<crate::git_workbench::HistoryFiles, FileError> {
    git_task(app, move |g| g.history_files(&id, &commit, start)).await
}

#[tauri::command]
async fn git_badges(
    app: tauri::AppHandle,
    entries: Vec<String>,
) -> Result<std::collections::HashMap<String, String>, FileError> {
    if entries.len() > 100 {
        return Err(FileError::new(
            "capacity",
            "Git badge request exceeds one library page.",
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<GitHost>();
        let git = host
            .0
            .lock()
            .map_err(|_| FileError::new("git-state", "Git unavailable."))?;
        if !git.selected() {
            return Ok(std::collections::HashMap::new());
        }
        let native = library(&app);
        let library = native
            .lock()
            .map_err(|_| FileError::new("state", "Library unavailable."))?;
        let paths = entries
            .into_iter()
            .filter_map(|id| {
                library
                    .scoped(&id)
                    .ok()
                    .map(|s| (id, s.canonical_path().to_owned()))
            })
            .collect();
        Ok(git.badges(paths))
    })
    .await
    .map_err(|_| FileError::new("git-task", "Git badge refresh interrupted."))?
}

struct ReviewHost(Mutex<crate::review::Reviews>);
async fn review_task<T: Send + 'static>(
    app: tauri::AppHandle,
    action: impl FnOnce(&mut crate::review::Reviews, &Documents) -> Result<T, FileError>
    + Send
    + 'static,
) -> Result<T, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = app.state::<ReviewHost>();
        let mut r = host
            .0
            .lock()
            .map_err(|_| FileError::new("review", "Review unavailable."))?;
        action(&mut r, &app.state::<Documents>())
    })
    .await
    .map_err(|_| FileError::new("review", "Review interrupted; durable records are kept."))?
}
fn review_capture(
    d: &Documents,
    id: &str,
    disk: bool,
) -> Result<crate::review::Capture, FileError> {
    let (path, bytes) = d.review_capture(id, disk)?;
    crate::review::Capture::new(&path, bytes)
}
fn review_verify(
    r: &crate::review::Reviews,
    d: &Documents,
    token: &str,
    id: &str,
) -> Result<(), FileError> {
    r.verify(
        token,
        &crate::scoped_file::path_key(d.scoped(id)?.canonical_path()),
    )
}
#[tauri::command]
async fn review_begin(app: tauri::AppHandle, id: String) -> Result<crate::review::View, FileError> {
    review_task(app, move |r, d| r.begin(review_capture(d, &id, false)?)).await
}
#[tauri::command]
async fn review_sync(
    app: tauri::AppHandle,
    token: String,
    id: String,
    disk: bool,
) -> Result<crate::review::View, FileError> {
    review_task(app, move |r, d| {
        review_verify(r, d, &token, &id)?;
        match review_capture(d, &id, disk) {
            Ok(c) => {
                if disk {
                    r.sync(&token, c)
                } else {
                    r.sync_grant(&token, c)
                }
            }
            Err(e) if disk && e.code == "missing" => r.missing(&token, d.renamed_name(&id)?),
            Err(e) => Err(e),
        }
    })
    .await
}
#[tauri::command]
async fn review_displayed(
    app: tauri::AppHandle,
    token: String,
    id: String,
) -> Result<(), FileError> {
    review_task(app, move |r, d| {
        let (key, revision) = d.review_identity(&id)?;
        r.displayed_revision(&token, &key, revision)
    })
    .await
}
#[tauri::command]
async fn review_end(app: tauri::AppHandle, token: String) -> Result<(), FileError> {
    review_task(app, move |r, _| r.end(&token)).await
}
#[tauri::command]
async fn review_update(
    app: tauri::AppHandle,
    token: String,
    id: String,
    version: u64,
    revision: String,
    action: crate::review::Action,
) -> Result<crate::review::View, FileError> {
    review_task(app, move |r, d| {
        review_verify(r, d, &token, &id)?;
        r.update_checked(&token, version, &revision, action)
    })
    .await
}
#[tauri::command]
async fn review_compare(
    app: tauri::AppHandle,
    token: String,
    id: String,
    baseline: String,
) -> Result<crate::review::Comparison, FileError> {
    review_task(app, move |r, d| {
        review_verify(r, d, &token, &id)?;
        r.compare(&token, &baseline)
    })
    .await
}
#[tauri::command]
async fn review_export(
    app: tauri::AppHandle,
    token: String,
    id: String,
    include_path: bool,
    snippets: bool,
) -> Result<String, FileError> {
    review_task(app, move |r, d| {
        review_verify(r, d, &token, &id)?;
        r.export_view(
            &token,
            if include_path {
                Some(
                    d.scoped(&id)?
                        .canonical_path()
                        .to_str().ok_or_else(||FileError::new("review","This native path is not UTF-8. Export with the file path redacted."))?
                        .to_owned(),
                )
            } else {
                None
            },
            snippets,
        )
    })
    .await
}
#[tauri::command]
async fn review_list(app: tauri::AppHandle) -> Result<Vec<crate::review::Entry>, FileError> {
    review_task(app, move |r, _| r.list()).await
}
#[tauri::command]
async fn review_delete(app: tauri::AppHandle, record: String) -> Result<(), FileError> {
    let deleted = record.clone();
    review_task(app.clone(), move |r, _| r.delete(&record)).await?;
    let _ = app.emit_to(
        "main",
        "review-deleted",
        serde_json::json!({"record":deleted}),
    );
    Ok(())
}

#[tauri::command]
async fn review_archive_export(
    app: tauri::AppHandle,
    record: String,
    snippets: bool,
) -> Result<String, FileError> {
    review_task(app, move |r, _| r.export_archive(&record, snippets)).await
}
fn feedback_format(json: String, format: &str) -> Result<String, FileError> {
    match format {
        "json" => Ok(json),
        "markdown" => {
            let n = json
                .split(|c| c != '`')
                .map(str::len)
                .max()
                .unwrap_or(0)
                .max(2)
                + 1;
            let fence = "`".repeat(n);
            Ok(format!(
                "# Document review feedback\n\nVerifiable saved-byte anchors; author attribution is not inferred.\n\n{fence}json\n{json}\n{fence}\n"
            ))
        }
        _ => Err(FileError::new(
            "review",
            "Choose JSON or Markdown feedback.",
        )),
    }
}
async fn save_review_report(
    app: tauri::AppHandle,
    json: String,
    format: String,
) -> Result<bool, FileError> {
    let report = feedback_format(json, &format)?;
    tauri::async_runtime::spawn_blocking(move || {
        #[cfg(feature = "e2e")]
        let selected = app
            .state::<crate::test_support::Fixtures>()
            .take_save()
            .map(tauri_plugin_dialog::FilePath::Path);
        #[cfg(not(feature = "e2e"))]
        let selected: Option<tauri_plugin_dialog::FilePath> = None;
        let selected = selected.or_else(|| {
            app.dialog()
                .file()
                .set_file_name(if format == "json" {
                    "review-feedback.json"
                } else {
                    "review-feedback.md"
                })
                .blocking_save_file()
        });
        let Some(path) = selected else {
            return Ok(false);
        };
        let path = path
            .into_path()
            .map_err(|_| FileError::new("review", "Choose a new local feedback destination."))?;
        let target = crate::scoped_file::ScopedFile::destination(&path)?;
        if editing_path_guard(&app)?
            .iter()
            .any(|p| target.canonical_path().starts_with(p))
        {
            return Err(FileError::new(
                "review",
                "Choose a destination outside app-owned resources.",
            ));
        }
        let p = crate::document::Provenance::new(report.as_bytes())
            .map_err(|_| FileError::new("review", "Cannot encode feedback."))?;
        crate::atomic_save::save_as(&target, &p.snapshot())?;
        Ok(true)
    })
    .await
    .map_err(|_| {
        FileError::new(
            "review",
            "Feedback export interrupted; review records are kept.",
        )
    })?
}
#[tauri::command]
async fn review_save(
    app: tauri::AppHandle,
    token: String,
    id: String,
    include_path: bool,
    snippets: bool,
    format: String,
) -> Result<bool, FileError> {
    let json = review_export(app.clone(), token, id, include_path, snippets).await?;
    save_review_report(app, json, format).await
}
#[tauri::command]
async fn review_archive_save(
    app: tauri::AppHandle,
    record: String,
    snippets: bool,
    format: String,
) -> Result<bool, FileError> {
    let json = review_archive_export(app.clone(), record, snippets).await?;
    save_review_report(app, json, format).await
}

// Selected local tools/databases and captures stay separate from Source writes.
use crate::power::{self, Kind, Role};
struct PowerHost(std::sync::Arc<Mutex<power::Powers>>);
fn powers(app: &tauri::AppHandle) -> std::sync::Arc<Mutex<power::Powers>> {
    app.state::<PowerHost>().0.clone()
}
fn power_confirm(app: &tauri::AppHandle, message: &str) -> bool {
    #[cfg(feature = "e2e")]
    if let Some(answer) = app
        .state::<crate::test_support::Fixtures>()
        .take_power_confirm(message)
    {
        return answer;
    }
    app.dialog()
        .message(message)
        .title("Confirm this local action")
        .kind(tauri_plugin_dialog::MessageDialogKind::Warning)
        .buttons(tauri_plugin_dialog::MessageDialogButtons::OkCancel)
        .blocking_show()
}
fn power_pick(
    app: &tauri::AppHandle,
    role: &str,
    folder: bool,
) -> Result<Option<std::path::PathBuf>, FileError> {
    #[cfg(feature = "e2e")]
    if let Some(path) = app
        .state::<crate::test_support::Fixtures>()
        .take_power_pick(role)
    {
        return Ok(Some(path));
    }
    let picker = app
        .dialog()
        .file()
        .set_title(format!("Select {role} explicitly"));
    let path = if folder {
        picker.blocking_pick_folder()
    } else {
        picker.blocking_pick_file()
    };
    path.map(|p| {
        p.into_path()
            .map_err(|_| power::err("Choose a local filesystem item."))
    })
    .transpose()
}
#[tauri::command]
fn power_state(app: tauri::AppHandle) -> Result<power::State, FileError> {
    Ok(powers(&app)
        .lock()
        .map_err(|_| power::err("Local jobs unavailable."))?
        .state())
}
#[tauri::command]
async fn power_enable(
    app: tauri::AppHandle,
    kind: Kind,
    on: bool,
) -> Result<power::State, FileError> {
    tauri::async_runtime::spawn_blocking(move||{
    if on&&!power_confirm(&app,&format!("Enable {kind:?} for this app session?\nThis does not trust a folder or run any document. Each run still needs native confirmation. Shell and local TeX retain normal user privileges; this is not an OS sandbox. Restart disables all runners.")){return Err(power::err("Enable canceled; no trust granted."));}
    let host=powers(&app);let mut p=host.lock().map_err(|_|power::err("Local jobs unavailable."))?;p.enable(kind,on);Ok(p.state())
}).await.map_err(|_|power::err("Runner enable interrupted."))?
}
#[tauri::command]
async fn power_choose_tool(
    app: tauri::AppHandle,
    role: Role,
) -> Result<Option<power::ToolView>, FileError> {
    tauri::async_runtime::spawn_blocking(move||{
    let Some(path)=power_pick(&app,&format!("tool-{role:?}").to_ascii_lowercase(),false)? else{return Ok(None)};
    let canonical=std::fs::canonicalize(&path)?;let host=powers(&app);let scratch={let p=host.lock().map_err(|_|power::err("Local jobs unavailable."))?;p.protect(&canonical)?;if p.tools.len()>=8{return Err(power::err("Eight tools selected; restart or reuse the current selection."));}p.scratch()?};
    if !power_confirm(&app,&format!("Use {} as {role:?}?\nOnly choose software you trust. Its version command will run now with normal user privileges. Tool selection does not enable document execution.",path.display())){return Ok(None)}
    let tool=std::sync::Arc::new(power::Tool::select(&path,role,&scratch.path)?);let view=tool.view.clone();{let mut p=host.lock().map_err(|_|power::err("Local jobs unavailable."))?;p.tools.retain(|_,t|t.view.role!=role);p.tools.insert(view.id.clone(),tool);}Ok(Some(view))
}).await.map_err(|_|power::err("Tool selection interrupted."))?
}
#[tauri::command]
async fn power_trust(
    app: tauri::AppHandle,
    kind: Kind,
) -> Result<Option<power::RootView>, FileError> {
    tauri::async_runtime::spawn_blocking(move||{
    let host=powers(&app);if !host.lock().map_err(|_|power::err("Local jobs unavailable."))?.enabled(kind){return Err(power::err("Enable this runner before trusting a folder."));}
    let Some(path)=power_pick(&app,&format!("root-{kind:?}").to_ascii_lowercase(),true)? else{return Ok(None)};
    let root=std::sync::Arc::new(power::Root::select(&path,kind)?);
    if !power_confirm(&app,&format!("Trust {} for {kind:?} in this session?\nOnly captured documents inside this folder can request this runner. Shell/TeX can still access other files with your normal OS privileges. A separate native prompt is required for every run.",root.path.display())){return Ok(None)}
    let mut p=host.lock().map_err(|_|power::err("Local jobs unavailable."))?;if !p.enabled(kind){return Err(power::err("Runner was disabled during selection."));}
    if p.roots.len()>=12{return Err(power::err("Twelve trusted folders reached; disable a runner to revoke its folders."));}p.protect(&root.path)?;let view=power::RootView{id:root.id.clone(),kind,name:root.path.display().to_string()};p.roots.retain(|_,r|r.kind!=kind);p.roots.insert(root.id.clone(),root);Ok(Some(view))
}).await.map_err(|_|power::err("Folder trust interrupted."))?
}
#[tauri::command]
async fn power_choose_database(app: tauri::AppHandle) -> Result<Option<power::DbView>, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = power_pick(&app, "database", false)? else {
            return Ok(None);
        };
        let db = std::sync::Arc::new(power::Database::select(&path)?);
        let host = powers(&app);
        let mut p = host
            .lock()
            .map_err(|_| power::err("Local jobs unavailable."))?;
        p.protect(&db.path)?;
        if p.databases.len() >= 4 {
            return Err(power::err(
                "Four database grants reached; restart to clear selection.",
            ));
        }
        let view = power::DbView {
            id: db.id.clone(),
            name: db.path.display().to_string(),
        };
        p.databases.clear();
        p.databases.insert(db.id.clone(), db);
        Ok(Some(view))
    })
    .await
    .map_err(|_| power::err("Database selection interrupted."))?
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PowerInput {
    document: Option<String>,
    editor: Option<String>,
    generation: Option<String>,
    revision: Option<u64>,
    saved: bool,
}
#[tauri::command]
async fn power_capture(
    app: tauri::AppHandle,
    input: PowerInput,
) -> Result<power::CaptureView, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let docs = app.state::<Documents>();
        let (name, bytes, path, origin, proof) = if input.saved {
            let id = input
                .document
                .ok_or_else(|| power::err("Select a saved document."))?;
            let (name, bytes, path) = docs.power_capture(&id)?;
            (
                name,
                bytes,
                path,
                "Saved native snapshot".to_string(),
                power::Proof::Saved(id),
            )
        } else {
            let id = input
                .editor
                .ok_or_else(|| power::err("No live working editor."))?;
            let generation = input
                .generation
                .ok_or_else(|| power::err("No live transport generation."))?;
            let revision = input
                .revision
                .ok_or_else(|| power::err("No working revision."))?;
            let host = app.state::<EditHost>();
            let guard = host
                .0
                .lock()
                .map_err(|_| power::err("Editor unavailable."))?;
            let service = guard.as_ref().map_err(Clone::clone)?;
            let (name, bytes, path) = service.power_capture(&id, &generation, revision)?;
            if let Some(document) = &input.document
                && path.as_ref().is_some_and(|p| {
                    docs.scoped(document)
                        .map(|d| d.canonical_path() != p)
                        .unwrap_or(true)
                })
            {
                return Err(power::err(
                    "Working snapshot belongs to another native document.",
                ));
            }
            (
                name,
                bytes,
                path,
                format!("Working native revision {revision}; saved file is unchanged"),
                power::Proof::Working {
                    id,
                    generation,
                    revision,
                },
            )
        };
        let mut c = power::Capture::new(name, bytes, origin, path, None)?;
        c.proof = Some(proof);
        powers(&app)
            .lock()
            .map_err(|_| power::err("Local jobs unavailable."))?
            .capture(c)
    })
    .await
    .map_err(|_| power::err("Document capture interrupted."))?
}
fn power_proof(app: &tauri::AppHandle, c: &power::Capture) -> Result<(), FileError> {
    match c
        .proof
        .as_ref()
        .ok_or_else(|| power::err("Capture proof unavailable."))?
    {
        power::Proof::Saved(id) => {
            let docs = app.state::<Documents>();
            if docs.status(id)? != "unchanged" {
                return Err(power::err(
                    "Saved file changed. Capture the document again before running.",
                ));
            }
            if power::hash(&docs.power_capture(id)?.1) != c.view.revision {
                return Err(power::err("Saved capture changed."));
            }
        }
        power::Proof::Working {
            id,
            generation,
            revision,
        } => {
            let host = app.state::<EditHost>();
            let guard = host
                .0
                .lock()
                .map_err(|_| power::err("Editor unavailable."))?;
            let bytes = guard
                .as_ref()
                .map_err(Clone::clone)?
                .power_capture(id, generation, *revision)?
                .1;
            if power::hash(&bytes) != c.view.revision {
                return Err(power::err("Working revision changed; capture again."));
            }
        }
    }
    Ok(())
}
#[tauri::command]
fn power_release(app: tauri::AppHandle, capture: String) -> Result<(), FileError> {
    powers(&app)
        .lock()
        .map_err(|_| power::err("Local jobs unavailable."))?
        .release(&capture);
    Ok(())
}
#[tauri::command]
async fn power_image(
    app: tauri::AppHandle,
    capture: String,
    reference: String,
) -> Result<bool, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        {
            let host = powers(&app);
            let p = host
                .lock()
                .map_err(|_| power::err("Local jobs unavailable."))?;
            let _capture = p
                .captures
                .get(&capture)
                .ok_or_else(|| power::err("Capture ended."))?;
            if reference.is_empty() || reference.len() > 2048 {
                return Err(power::err(
                    "Image reference is outside this captured document.",
                ));
            }
        }
        let Some(path) = power_pick(&app, "image", false)? else {
            return Ok(false);
        };
        let image = crate::reader_assets::image(&path)?;
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD
            .decode(image.data_url.split_once(',').unwrap().1)
            .map_err(|_| power::err("Invalid raster."))?;
        power_asset(&app, &capture, reference, data)?;
        Ok(true)
    })
    .await
    .map_err(|_| power::err("Image selection interrupted."))?
}
fn power_asset(
    app: &tauri::AppHandle,
    capture: &str,
    key: String,
    bytes: Vec<u8>,
) -> Result<(), FileError> {
    crate::reader_assets::image_bytes(&bytes)?;
    let host = powers(app);
    let mut p = host
        .lock()
        .map_err(|_| power::err("Local jobs unavailable."))?;
    let c = p
        .captures
        .get_mut(capture)
        .ok_or_else(|| power::err("Capture ended."))?;
    let c = std::sync::Arc::make_mut(c);
    if c.assets.len() >= 16
        || c.assets.values().map(Vec::len).sum::<usize>() + bytes.len() > 24 * 1024 * 1024
    {
        return Err(power::err("PDF assets exceed 16 entries / 24 MiB."));
    }
    c.assets.insert(key, bytes);
    Ok(())
}
#[tauri::command]
fn power_diagram(
    app: tauri::AppHandle,
    capture: String,
    source: String,
    png: String,
) -> Result<(), FileError> {
    if source.len() > 65536 || png.len() > 12 * 1024 * 1024 {
        return Err(power::err("Diagram exceeds its export budget."));
    }
    {
        let host = powers(&app);
        let p = host
            .lock()
            .map_err(|_| power::err("Local jobs unavailable."))?;
        let c = p
            .captures
            .get(&capture)
            .ok_or_else(|| power::err("Capture ended."))?;
        if !c.view.text.contains(source.trim()) {
            return Err(power::err("Diagram source is outside this capture."));
        }
    }
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png)
        .map_err(|_| power::err("Invalid rendered PNG."))?;
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(power::err("Rendered diagram must be PNG."));
    }
    power_asset(
        &app,
        &capture,
        format!("mermaid:{}", power::hash(source.trim().as_bytes())),
        bytes,
    )
}
#[derive(Serialize)]
struct PowerApproval {
    token: String,
    summary: String,
}
#[tauri::command]
async fn power_prepare(
    app: tauri::AppHandle,
    request: power::Request,
) -> Result<PowerApproval, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        if request.mode.kind().is_some() {
            let c = {
                powers(&app)
                    .lock()
                    .map_err(|_| power::err("Local jobs unavailable."))?
                    .captures
                    .get(&request.capture)
                    .cloned()
                    .ok_or_else(|| power::err("Capture ended."))?
            };
            power_proof(&app, &c)?;
        }
        let (token, summary) = powers(&app)
            .lock()
            .map_err(|_| power::err("Local jobs unavailable."))?
            .prepare(request)?;
        Ok(PowerApproval { token, summary })
    })
    .await
    .map_err(|_| power::err("Job preparation interrupted."))?
}
#[tauri::command]
async fn power_run(app: tauri::AppHandle, token: String) -> Result<Option<String>, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = powers(&app);
        let p = host
            .lock()
            .map_err(|_| power::err("Local jobs unavailable."))?
            .take(&token)?;
        let scratch=host.lock().map_err(|_|power::err("Local jobs unavailable."))?.scratch()?;
        let cwd=if p.request.mode==power::Mode::Shell {p.root.as_ref().unwrap().path.clone()} else {scratch.path.clone()};
        let summary=format!("{}\n\nWorking directory: {}\nConverter: {}\nLimits: 256 KiB per stream; shell/TeX 30 seconds per process; SQLite 5 seconds. Attached process-group cancellation is best effort; detached descendants may survive.",p.summary,cwd.display(),p.pandoc.as_ref().map(|t|format!("{} ({})",t.path.display(),t.view.version)).unwrap_or_else(||"None".into()));
        if !power_confirm(&app, &summary) { return Ok(None); }

        if p.request.mode.kind().is_some() {
            power_proof(&app, &p.capture)?;
        }
        let job = std::sync::Arc::new(power::Job::new(p.request.mode.kind()));
        let id = uuid::Uuid::new_v4().to_string();
        let scratch = {
            let mut service = host
                .lock()
                .map_err(|_| power::err("Local jobs unavailable."))?;
            if !service.captures.contains_key(&p.request.capture)
                || p.request.mode.kind().is_some_and(|k| !service.enabled(k))
                || p.root
                    .as_ref()
                    .is_some_and(|r| !service.roots.contains_key(&r.id))
                || p.tool.as_ref().is_some_and(|t|!service.tools.contains_key(&t.view.id))
                || p.pandoc.as_ref().is_some_and(|t|!service.tools.contains_key(&t.view.id))
                || p.database.as_ref().is_some_and(|d|!service.databases.contains_key(&d.id))
            {
                return Err(power::err("Authority revoked while confirmation was open."));
            }
            if service
                .jobs
                .values()
                .any(|j| j.result.lock().unwrap().status == "running")
            {
                return Err(power::err(
                    "One local job at a time. Cancel or wait for the current job.",
                ));
            }
            service.jobs.insert(id.clone(), job.clone());
            scratch
        };
        tauri::async_runtime::spawn_blocking(move || {
            let result = power::execute(p, scratch, &job);
            job.finish(result);
        });
        Ok(Some(id))
    })
    .await
    .map_err(|_| power::err("Job confirmation interrupted."))?
}
#[tauri::command]
fn power_poll(app: tauri::AppHandle, job: String) -> Result<power::ResultView, FileError> {
    let host = powers(&app);
    let p = host
        .lock()
        .map_err(|_| power::err("Local jobs unavailable."))?;
    let j = p
        .jobs
        .get(&job)
        .ok_or_else(|| power::err("Unknown job handle."))?;
    let result = j
        .result
        .lock()
        .map_err(|_| power::err("Job result unavailable."))?
        .clone();
    Ok(result)
}
#[tauri::command]
fn power_cancel(app: tauri::AppHandle, job: String) -> Result<(), FileError> {
    let host = powers(&app);
    let p = host
        .lock()
        .map_err(|_| power::err("Local jobs unavailable."))?;
    p.jobs
        .get(&job)
        .ok_or_else(|| power::err("Unknown job handle."))?
        .cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}
#[tauri::command]
fn power_clear(app: tauri::AppHandle) -> Result<(), FileError> {
    powers(&app)
        .lock()
        .map_err(|_| power::err("Local jobs unavailable."))?
        .clear();
    Ok(())
}
#[tauri::command]
async fn power_save_pdf(app: tauri::AppHandle, job: String) -> Result<Option<String>, FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        let host = powers(&app);
        let j = host
            .lock()
            .map_err(|_| power::err("Local jobs unavailable."))?
            .jobs
            .get(&job)
            .cloned()
            .ok_or_else(|| power::err("Unknown job."))?;
        let bytes = j
            .pdf
            .lock()
            .map_err(|_| power::err("PDF unavailable."))?
            .clone()
            .ok_or_else(|| power::err("No PDF result."))?;
        #[cfg(feature = "e2e")]
        let injected = app
            .state::<crate::test_support::Fixtures>()
            .take_power_pick("pdf-destination");
        #[cfg(not(feature = "e2e"))]
        let injected: Option<std::path::PathBuf> = None;
        let path = if let Some(path) = injected {
            path
        } else {
            let Some(path) = app
                .dialog()
                .file()
                .set_title("Export PDF to a new file")
                .set_file_name("document.pdf")
                .add_filter("PDF", &["pdf"])
                .blocking_save_file()
            else {
                return Ok(None);
            };
            path.into_path()
                .map_err(|_| power::err("Choose a local new PDF destination."))?
        };
        let target = crate::scoped_file::ScopedFile::destination(&path)?;
        host.lock()
            .map_err(|_| power::err("Local jobs unavailable."))?
            .protect(target.canonical_path())?;
        let committed = crate::atomic_save::write_asset(&target, &bytes)?;
        let path = target.canonical_path().to_owned();
        let scoped = crate::scoped_file::ScopedFile::selected(&path)?;
        let id = uuid::Uuid::new_v4().to_string();
        host.lock()
            .map_err(|_| power::err("Local jobs unavailable."))?
            .exports
            .insert(id.clone(), (scoped, committed.hash));
        Ok(Some(id))
    })
    .await
    .map_err(|_| power::err("PDF export interrupted."))?
}
#[tauri::command]
async fn power_open_pdf(app: tauri::AppHandle, export: String) -> Result<(), FileError> {
    tauri::async_runtime::spawn_blocking(move || {
        use std::io::Read;
        let host = powers(&app);
        let p = host
            .lock()
            .map_err(|_| power::err("Local jobs unavailable."))?;
        let (scoped, expected) = p
            .exports
            .get(&export)
            .ok_or_else(|| power::err("Unknown exported PDF."))?;
        scoped.validate_selection()?;
        let (mut f, _, _) = scoped.snapshot_bounded(32 * 1024 * 1024)?;
        let mut b = vec![];
        f.read_to_end(&mut b)?;
        if power::hash(&b) != *expected || !b.starts_with(b"%PDF-") {
            return Err(power::err(
                "Exported PDF changed. Export again before opening it.",
            ));
        }
        let path = scoped.canonical_path().to_owned();
        drop(p);
        #[cfg(target_os = "macos")]
        let result = std::process::Command::new("/usr/bin/open")
            .args(["-a", "Preview"])
            .arg(path)
            .status();
        #[cfg(target_os = "linux")]
        let result = std::process::Command::new("/usr/bin/xdg-open")
            .arg(path)
            .status();
        if !result.map(|s| s.success()).unwrap_or(false) {
            return Err(power::err(
                "System PDF viewer could not open. Open the exported file manually to print.",
            ));
        }
        Ok(())
    })
    .await
    .map_err(|_| power::err("Open PDF interrupted."))?
}
