#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

//! The Linux text widget is a view of the existing source document.
//! Persistence, revision history and Markdown commands stay in SourceEditor.

use serde::{Deserialize, Serialize};
use tauri::WebviewWindow;

#[derive(Deserialize, Serialize)]
pub(crate) struct Edit {
    from: i32,
    to: i32,
    text: String,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
pub(crate) struct Selection {
    anchor: i32,
    head: i32,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
pub(crate) struct Anchor {
    offset: i32,
    fraction: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Appearance {
    ink: String,
    paper: String,
    selection: String,
    quiet: String,
    size: i32,
    spacing: f64,
    wrap: bool,
    numbers: bool,
    tab_width: i32,
}

#[derive(Clone, Copy, Deserialize)]
pub(crate) struct Rectangle {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Default, Deserialize)]
pub(crate) struct Update {
    edits: Vec<Edit>,
    selection: Option<Selection>,
    reveal: Option<Anchor>,
    editable: bool,
    focus: bool,
}

#[derive(Clone, Serialize)]
pub(crate) struct Key {
    key: String,
    ctrl: bool,
    shift: bool,
    alt: bool,
}

#[derive(Serialize)]
pub(crate) struct Reply {
    accepted: bool,
    edits: Vec<Edit>,
    selection: Selection,
    anchor: Anchor,
    keys: Vec<Key>,
    focused: bool,
}

#[tauri::command]
pub(crate) fn native_source_open(
    window: WebviewWindow,
    token: String,
    text: String,
    selection: Selection,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        gtk_source::open(window, token, text, selection)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (window, token, text, selection);
        Err("Native source editing uses GTK on Linux.".into())
    }
}

#[tauri::command]
pub(crate) fn native_source_layout(
    token: String,
    rect: Option<Rectangle>,
    appearance: Option<Appearance>,
) -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        gtk_source::layout(&token, rect, appearance)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (token, rect, appearance);
        Ok(false)
    }
}

#[tauri::command]
pub(crate) fn native_source_sync(token: String, update: Update) -> Result<Reply, String> {
    #[cfg(target_os = "linux")]
    {
        gtk_source::sync(&token, update)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (token, update);
        Err("Native source editing uses GTK on Linux.".into())
    }
}

#[tauri::command]
pub(crate) fn native_source_close(token: String) {
    #[cfg(target_os = "linux")]
    gtk_source::close(&token);
    #[cfg(not(target_os = "linux"))]
    let _ = token;
}

#[cfg(target_os = "linux")]
mod gtk_source {
    use super::*;
    use gtk::{gdk, glib, pango, prelude::*};
    use std::{
        cell::{Cell, RefCell},
        collections::HashMap,
        rc::Rc,
    };
    use tauri::{Emitter, Manager};

    #[derive(Default)]
    struct Events {
        edits: Vec<Edit>,
        keys: Vec<Key>,
        scheduled: bool,
        syncing: bool,
    }

    struct Editor {
        view: gtk::TextView,
        scroll: gtk::ScrolledWindow,
        buffer: gtk::TextBuffer,
        styles: gtk::CssProvider,
        rect: Cell<Option<Rectangle>>,
        events: Rc<RefCell<Events>>,
    }

    #[derive(Default)]
    struct Host {
        surface: Option<gtk::Overlay>,
        editors: HashMap<String, Editor>,
        font_ready: bool,
    }

    thread_local! {
        static HOST: RefCell<Host> = RefCell::new(Host::default());
    }

    fn select(buffer: &gtk::TextBuffer, selected: Selection) {
        let anchor = buffer.iter_at_offset(selected.anchor.clamp(0, buffer.char_count()));
        let head = buffer.iter_at_offset(selected.head.clamp(0, buffer.char_count()));
        buffer.select_range(&head, &anchor);
    }

    fn register_font(window: &WebviewWindow) -> Result<(), String> {
        use std::ffi::{CString, c_char, c_void};
        use std::os::unix::ffi::OsStrExt;

        #[link(name = "fontconfig")]
        unsafe extern "C" {
            fn FcConfigGetCurrent() -> *mut c_void;
            fn FcConfigParseAndLoadFromMemory(
                config: *mut c_void,
                rule: *const c_char,
                complain: i32,
            ) -> i32;
            fn FcConfigAppFontAddFile(config: *mut c_void, path: *const c_char) -> i32;
        }

        let path = window
            .path()
            .app_cache_dir()
            .map_err(|e| e.to_string())?
            .join("bundled-fonts");
        std::fs::create_dir_all(&path).map_err(|e| e.to_string())?;
        let font = path.join("MarktubNative-Amiri-Regular.ttf");
        let bytes = include_bytes!("../../src/fonts/assets/Amiri-Regular.ttf");
        if !(std::fs::metadata(&font).is_ok_and(|info| info.len() == bytes.len() as u64)
            && std::fs::read(&font).is_ok_and(|current| current == bytes))
        {
            let temporary = path.join(format!("Amiri-{}.ttf", uuid::Uuid::new_v4()));
            std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
            std::fs::rename(&temporary, &font).map_err(|e| e.to_string())?;
        }
        let name = CString::new(font.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
        // Give the bundled face its own name so an installed Amiri version cannot
        // silently replace the font used by the reader's Arabic compatibility face.
        let (comparison, file) = font
            .to_str()
            .map(|path| ("eq", path))
            .unwrap_or(("contains", "MarktubNative-Amiri-Regular.ttf"));
        let file = glib::markup_escape_text(file);
        let rule = CString::new(format!(
            r#"<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd"><fontconfig><match target="scan"><test name="file" compare="{comparison}"><string>{file}</string></test><edit name="family" mode="prepend"><string>Marktub Native Amiri</string></edit></match></fontconfig>"#
        )).map_err(|e| e.to_string())?;
        let registered = unsafe {
            let config = FcConfigGetCurrent();
            !config.is_null()
                && FcConfigParseAndLoadFromMemory(config, rule.as_ptr(), 1) != 0
                && FcConfigAppFontAddFile(config, name.as_ptr()) != 0
        };
        if !registered {
            return Err("GTK could not load the bundled Arabic font.".into());
        }
        Ok(())
    }

    fn surface(window: &WebviewWindow) -> Result<gtk::Overlay, String> {
        let container = window.default_vbox().map_err(|e| e.to_string())?;
        let web = container
            .children()
            .pop()
            .ok_or("The Markdown workspace has no WebView.")?;
        let surface = gtk::Overlay::new();
        // GtkOverlay's default alignment uses a child's preferred size. The
        // source scroller must occupy exactly the WebView's editor rectangle.
        unsafe {
            glib::gobject_ffi::g_signal_connect_data(
                surface.as_ptr().cast(),
                c"get-child-position".as_ptr(),
                Some(std::mem::transmute::<
                    unsafe extern "C" fn(
                        *mut gtk::ffi::GtkOverlay,
                        *mut gtk::ffi::GtkWidget,
                        *mut gdk::ffi::GdkRectangle,
                        glib::ffi::gpointer,
                    ) -> glib::ffi::gboolean,
                    unsafe extern "C" fn(),
                >(child_position)),
                std::ptr::null_mut(),
                None,
                0,
            );
        }
        container.remove(&web);
        surface.add(&web);
        container.pack_start(&surface, true, true, 0);
        surface.show();
        Ok(surface)
    }

    unsafe extern "C" fn child_position(
        _overlay: *mut gtk::ffi::GtkOverlay,
        child: *mut gtk::ffi::GtkWidget,
        allocation: *mut gdk::ffi::GdkRectangle,
        _data: glib::ffi::gpointer,
    ) -> glib::ffi::gboolean {
        let rect = HOST.with(|host| {
            let host = host.try_borrow().ok()?;
            host.editors
                .values()
                .find(|editor| editor.scroll.as_ptr().cast() == child)?
                .rect
                .get()
        });
        let Some(rect) = rect else {
            return glib::ffi::GFALSE;
        };
        if let Some(allocation) = unsafe { allocation.as_mut() } {
            allocation.x = rect.x;
            allocation.y = rect.y;
            allocation.width = rect.width;
            allocation.height = rect.height;
            glib::ffi::GTRUE
        } else {
            glib::ffi::GFALSE
        }
    }

    pub(super) fn open(
        window: WebviewWindow,
        token: String,
        text: String,
        selection: Selection,
    ) -> Result<(), String> {
        if !gtk::is_initialized_main_thread() || window.label() != "main" {
            return Err("GTK source editing requires the main window thread.".into());
        }
        if text.contains('\0') {
            return Err("GTK source editing requires text without NUL characters.".into());
        }
        HOST.with_borrow_mut(|host| {
            if host.editors.contains_key(&token) {
                return Ok(());
            }
            if !host.font_ready {
                register_font(&window)?;
                host.font_ready = true;
            }
            if host.surface.is_none() {
                host.surface = Some(surface(&window)?);
            }
            let buffer = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
            buffer.set_text(&text);
            select(&buffer, selection);
            let view = gtk::TextView::with_buffer(&buffer);
            view.set_left_margin(12);
            view.set_right_margin(12);
            view.set_top_margin(12);
            view.set_bottom_margin(12);
            view.set_accepts_tab(true);
            if let Some(accessible) = view.accessible() {
                gtk::atk::prelude::AtkObjectExt::set_name(&accessible, "Source Markdown editor");
            }
            view.connect_local("draw", true, |values| {
                let view = values[0].get::<gtk::TextView>().ok()?;
                let cairo = values[1].get::<gtk::cairo::Context>().ok()?;
                draw_numbers(&view, &cairo);
                Some(false.to_value())
            });
            let styles = gtk::CssProvider::new();
            view.style_context()
                .add_provider(&styles, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION);
            let scroll =
                gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
            scroll.set_direction(gtk::TextDirection::Ltr);
            scroll.set_no_show_all(true);
            scroll.set_overlay_scrolling(false);
            scroll.add(&view);
            view.show();
            host.surface.as_ref().unwrap().add_overlay(&scroll);
            let events = Rc::new(RefCell::new(Events::default()));
            connect(&window, &token, &view, &buffer, &scroll, &events);
            host.editors.insert(
                token,
                Editor {
                    view,
                    scroll,
                    buffer,
                    styles,
                    rect: Cell::new(None),
                    events,
                },
            );
            Ok(())
        })
    }

    fn connect(
        window: &WebviewWindow,
        token: &str,
        view: &gtk::TextView,
        buffer: &gtk::TextBuffer,
        scroll: &gtk::ScrolledWindow,
        events: &Rc<RefCell<Events>>,
    ) {
        let window = window.clone();
        let token = token.to_string();
        let pending = events.clone();
        let signal = move || {
            let mut events = pending.borrow_mut();
            if !events.syncing && !events.scheduled {
                events.scheduled = true;
                let pending = pending.clone();
                let window = window.clone();
                let token = token.clone();
                glib::idle_add_local_once(move || {
                    pending.borrow_mut().scheduled = false;
                    let _ = window.emit("native-source-change", token);
                });
            }
        };
        let pending = events.clone();
        buffer.connect_insert_text(move |buffer, at, text| {
            // Keep the GTK buffer and the normalized source projection on the
            // same offsets after a paste from an editor that uses CRLF.
            if text.contains('\r') && !pending.borrow().syncing {
                let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                buffer.stop_signal_emission_by_name("insert-text");
                buffer.insert(at, &normalized);
                return;
            }
            let mut pending = pending.borrow_mut();
            if !pending.syncing {
                let at = at.offset();
                pending.edits.push(Edit {
                    from: at,
                    to: at,
                    text: text.into(),
                });
            }
        });
        let pending = events.clone();
        buffer.connect_delete_range(move |_, from, to| {
            let mut pending = pending.borrow_mut();
            if !pending.syncing {
                pending.edits.push(Edit {
                    from: from.offset(),
                    to: to.offset(),
                    text: String::new(),
                });
            }
        });
        let notify = signal.clone();
        buffer.connect_changed(move |_| notify());
        let notify = signal.clone();
        buffer.connect_mark_set(move |_, _, mark| {
            if mark
                .name()
                .is_some_and(|name| name == "insert" || name == "selection_bound")
            {
                notify();
            }
        });
        let notify = signal.clone();
        scroll
            .vadjustment()
            .connect_value_changed(move |_| notify());
        let pending = events.clone();
        view.connect_key_press_event(move |view, event| {
            let state = event.state();
            let ctrl = state.contains(gdk::ModifierType::CONTROL_MASK);
            let alt = state.contains(gdk::ModifierType::MOD1_MASK);
            let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
            let key = event.keyval();
            let name = key.name().unwrap_or_default();
            // Keep app shortcuts available while a non-Latin layout is active.
            // Use the same physical key from a configured Latin layout.
            let mut character = key.to_unicode();
            if (ctrl || alt) && character.is_some_and(|value| !value.is_ascii()) {
                character = gdk::Keymap::for_display(&view.display())
                    .and_then(|keymap| {
                        keymap
                            .entries_for_keycode(u32::from(event.hardware_keycode()))
                            .into_iter()
                            .filter(|(entry, _)| entry.level() == 0)
                            .filter_map(|(_, value)| gdk::keys::Key::from(value).to_unicode())
                            .find(|value| value.is_ascii_graphic())
                    })
                    .or(character);
            }
            let key = match name.as_str() {
                "Left" | "Right" | "Up" | "Down" => format!("Arrow{name}"),
                "ISO_Left_Tab" => "Tab".into(),
                "BackSpace" => "Backspace".into(),
                _ => character
                    .filter(|key| !key.is_control())
                    .map_or_else(|| name.to_string(), |key| key.to_string()),
            };
            // Keep the native word-navigation and clipboard bindings. Alt
            // arrows and Ctrl+Tab are application navigation in Marktub.
            let native = ctrl
                && !alt
                && matches!(
                    key.to_ascii_lowercase().as_str(),
                    "a" | "c"
                        | "v"
                        | "x"
                        | "arrowleft"
                        | "arrowright"
                        | "arrowup"
                        | "arrowdown"
                        | "home"
                        | "end"
                        | "backspace"
                        | "delete"
                );
            if (ctrl || alt)
                && !key.is_empty()
                && !native
                && !name.ends_with("_L")
                && !name.ends_with("_R")
                && !state.contains(gdk::ModifierType::MOD5_MASK)
            {
                pending.borrow_mut().keys.push(Key {
                    key,
                    ctrl,
                    shift,
                    alt,
                });
                signal();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }

    fn draw_numbers(view: &gtk::TextView, cairo: &gtk::cairo::Context) -> glib::Propagation {
        let Some(gutter) = gtk::prelude::TextViewExt::window(view, gtk::TextWindowType::Left)
        else {
            return glib::Propagation::Proceed;
        };
        if !gtk::cairo_should_draw_window(cairo, &gutter) {
            return glib::Propagation::Proceed;
        }
        let visible = view.visible_rect();
        let (mut line, _) = view.line_at_y(visible.y());
        let layout = view.create_pango_layout(None);
        layout.set_font_description(Some(&pango::FontDescription::from_string("Monospace 9")));
        if cairo.save().is_ok() {
            gtk::cairo_transform_to_window(cairo, view, &gutter);
            loop {
                let (y, height) = view.line_yrange(&line);
                if y >= visible.y() + visible.height() {
                    break;
                }
                layout.set_text(&(line.line() + 1).to_string());
                let (width, text_height) = layout.pixel_size();
                let (_, y) = view.buffer_to_window_coords(gtk::TextWindowType::Left, 0, y);
                gtk::render_layout(
                    &view.style_context(),
                    cairo,
                    f64::from(43 - width),
                    f64::from(y + (height.min(text_height + 8) - text_height) / 2),
                    &layout,
                );
                if !line.forward_line() {
                    break;
                }
            }
            let _ = cairo.restore();
        }
        glib::Propagation::Proceed
    }

    fn color(input: &str, fallback: &'static str) -> String {
        if input.len() == 7
            && input.starts_with('#')
            && input.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
        {
            input.into()
        } else {
            fallback.into()
        }
    }

    fn decorate(editor: &Editor, style: Appearance) -> Result<(), String> {
        let size = style.size.clamp(11, 40);
        let ink = color(&style.ink, "#242424");
        let paper = color(&style.paper, "#ffffff");
        let selection = color(&style.selection, "#9bc5ee");
        let quiet = color(&style.quiet, "#616161");
        let css = format!(
            "textview, textview text {{ font-family: \"Marktub Native Amiri\", serif; font-size: {size}px; color: {ink}; background-color: {paper}; caret-color: {ink}; }} textview text selection {{ color: {ink}; background-color: {selection}; }} textview border {{ color: {quiet}; background-color: {paper}; }}"
        );
        editor
            .styles
            .load_from_data(css.as_bytes())
            .map_err(|e| e.to_string())?;
        editor.view.set_wrap_mode(if style.wrap {
            gtk::WrapMode::WordChar
        } else {
            gtk::WrapMode::None
        });
        editor.scroll.set_policy(
            if style.wrap {
                gtk::PolicyType::Never
            } else {
                gtk::PolicyType::Automatic
            },
            gtk::PolicyType::Automatic,
        );
        let leading = ((style.spacing.clamp(1.2, 2.4) - 1.5) * f64::from(size))
            .max(0.0)
            .round() as i32;
        editor.view.set_pixels_above_lines(leading / 2);
        editor.view.set_pixels_below_lines(leading - leading / 2);
        editor.view.set_pixels_inside_wrap(leading);
        let mut tabs = pango::TabArray::new(1, true);
        tabs.set_tab(
            0,
            pango::TabAlign::Left,
            (style.tab_width.clamp(1, 12) * size) / 2,
        );
        editor.view.set_tabs(&tabs);
        editor.view.set_border_window_size(
            gtk::TextWindowType::Left,
            if style.numbers { 50 } else { 0 },
        );
        editor.view.queue_draw();
        Ok(())
    }

    pub(super) fn layout(
        token: &str,
        rect: Option<Rectangle>,
        appearance: Option<Appearance>,
    ) -> Result<bool, String> {
        HOST.with_borrow(|host| {
            let Some(editor) = host.editors.get(token) else {
                return Ok(false);
            };
            if let Some(style) = appearance {
                decorate(editor, style)?;
            }
            editor
                .rect
                .set(rect.filter(|r| r.width > 0 && r.height > 0));
            let visible = editor.rect.get().is_some();
            editor.scroll.set_visible(visible);
            if let Some(surface) = &host.surface {
                surface.queue_resize();
            }
            Ok(visible)
        })
    }

    fn apply(editor: &Editor, edits: &[Edit]) -> Result<(), String> {
        let buffer = &editor.buffer;
        let mut ranges = Vec::with_capacity(edits.len());
        let mut end = 0;
        for edit in edits {
            let (from, to) = (edit.from, edit.to);
            if from < end || to < from || to > buffer.char_count() || edit.text.contains('\0') {
                return Err("GTK received an invalid source change.".into());
            }
            ranges.push((from, to, &edit.text));
            end = to;
        }
        for (from, to, text) in ranges.into_iter().rev() {
            let mut start = buffer.iter_at_offset(from);
            if from < to {
                buffer.delete(&mut start, &mut buffer.iter_at_offset(to));
            }
            if !text.is_empty() {
                buffer.insert(&mut start, text);
            }
        }
        Ok(())
    }

    pub(super) fn sync(token: &str, update: Update) -> Result<Reply, String> {
        HOST.with_borrow(|host| {
            let editor = host
                .editors
                .get(token)
                .ok_or("The GTK source pane is closed.")?;
            let (edits, keys) = {
                let mut events = editor.events.borrow_mut();
                events.syncing = true;
                (
                    std::mem::take(&mut events.edits),
                    std::mem::take(&mut events.keys),
                )
            };
            let accepted = edits.is_empty();
            let result = if accepted {
                apply(editor, &update.edits)
            } else {
                Ok(())
            };
            if accepted && result.is_ok() {
                if let Some(selection) = update.selection {
                    select(&editor.buffer, selection);
                    if update.focus {
                        editor
                            .view
                            .scroll_mark_onscreen(&editor.buffer.get_insert().unwrap());
                    }
                }
                if let Some(anchor) = update.reveal {
                    let line = editor
                        .buffer
                        .iter_at_offset(anchor.offset.clamp(0, editor.buffer.char_count()));
                    let (top, height) = editor.view.line_yrange(&line);
                    let adjustment = editor.scroll.vadjustment();
                    adjustment.set_value(
                        f64::from(top) + anchor.fraction.clamp(0.0, 1.0) * f64::from(height),
                    );
                }
            }
            editor.view.set_editable(update.editable);
            if update.focus && editor.scroll.is_visible() {
                editor.view.grab_focus();
            }
            editor.events.borrow_mut().syncing = false;
            result?;
            let buffer = &editor.buffer;
            let head = buffer.iter_at_mark(&buffer.get_insert().unwrap());
            let anchor = buffer.iter_at_mark(&buffer.selection_bound().unwrap());
            let y = editor.view.visible_rect().y();
            let (top, begin) = editor.view.line_at_y(y);
            let (_, height) = editor.view.line_yrange(&top);
            Ok(Reply {
                accepted,
                edits,
                keys,
                selection: Selection {
                    anchor: anchor.offset(),
                    head: head.offset(),
                },
                anchor: Anchor {
                    offset: top.offset(),
                    fraction: (f64::from(y - begin) / f64::from(height.max(1))).clamp(0.0, 1.0),
                },
                focused: editor.view.has_focus(),
            })
        })
    }

    #[cfg(feature = "e2e")]
    pub(super) fn fixture(action: &str, text: &str) -> Result<String, String> {
        HOST.with_borrow(|host| {
            let editor = host.editors.values().find(|e| e.scroll.is_visible());
            let Some(editor) = editor else {
                return if action == "native-source-status" {
                    Ok("null".into())
                } else {
                    Err("There is no visible GTK source pane.".into())
                };
            };
            let buffer = &editor.buffer;
            if action == "native-source-screenshot" {
                use base64::Engine;
                let window = editor
                    .view
                    .toplevel()
                    .and_then(|widget| widget.window())
                    .ok_or("The GTK workspace has no window.")?;
                let screenshot = window
                    .pixbuf(0, 0, window.width(), window.height())
                    .ok_or("The GTK workspace could not be captured.")?;
                let png = screenshot
                    .save_to_bufferv("png", &[])
                    .map_err(|error| error.to_string())?;
                return Ok(base64::engine::general_purpose::STANDARD.encode(png));
            }
            if action == "native-source-select" {
                let selection: Selection =
                    serde_json::from_str(text).map_err(|error| error.to_string())?;
                select(buffer, selection);
                editor.view.grab_focus();
            }
            if action == "native-source-insert" {
                #[derive(Deserialize)]
                struct Input {
                    selection: Selection,
                    text: String,
                }
                let input: Input = serde_json::from_str(text).map_err(|error| error.to_string())?;
                select(buffer, input.selection);
                editor.view.grab_focus();
                editor
                    .view
                    .emit_by_name::<()>("insert-at-cursor", &[&input.text]);
            }
            let text = buffer
                .text(&buffer.start_iter(), &buffer.end_iter(), true)
                .unwrap_or_default();
            let allocation = editor.scroll.allocation();
            Ok(serde_json::json!({
                "text": text.as_str(),
                "x": allocation.x(),
                "y": allocation.y(),
                "width": allocation.width(),
                "height": allocation.height(),
                "focused": editor.view.has_focus(),
            })
            .to_string())
        })
    }

    pub(super) fn close(token: &str) {
        HOST.with_borrow_mut(|host| {
            if let Some(editor) = host.editors.remove(token)
                && let Some(surface) = &host.surface
            {
                surface.remove(&editor.scroll);
            }
        });
    }
}

#[cfg(all(target_os = "linux", feature = "e2e"))]
pub(crate) fn fixture(action: &str, text: &str) -> Result<String, String> {
    gtk_source::fixture(action, text)
}
