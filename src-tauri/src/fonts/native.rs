//! Fontconfig registration for the native GTK source view.
use gtk::glib;
use std::{
    ffi::{CString, c_char, c_void},
    os::unix::ffi::OsStrExt,
};
use tauri::{Manager, WebviewWindow};

pub const ARABIC_FAMILY: &str = "Marktub Native Amiri";
const FACES: &[(&str, &str, &[u8])] = &[
    (
        "Amiri",
        ARABIC_FAMILY,
        include_bytes!("../../../src/fonts/assets/Amiri-Regular.ttf"),
    ),
    (
        "Cousine",
        "Marktub Native Cousine",
        include_bytes!("../../../src/fonts/assets/Cousine-Regular.ttf"),
    ),
    (
        "Noto Naskh Arabic",
        "Marktub Native Noto Naskh Arabic",
        include_bytes!("../../../src/fonts/assets/NotoNaskhArabic[wght].ttf"),
    ),
    (
        "Noto Sans Arabic",
        "Marktub Native Noto Sans Arabic",
        include_bytes!("../../../src/fonts/assets/NotoSansArabic[wdth,wght].ttf"),
    ),
];

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

pub fn register(window: &WebviewWindow) -> Result<(), String> {
    let directory = window
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("bundled-fonts");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    let mut rules = String::from(
        r#"<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd"><fontconfig>"#,
    );
    for &(_, alias, bytes) in FACES {
        let filename = format!("{alias}-Regular.ttf");
        let font = directory.join(&filename);
        if !(std::fs::metadata(&font).is_ok_and(|info| info.len() == bytes.len() as u64)
            && std::fs::read(&font).is_ok_and(|current| current == bytes))
        {
            let temporary = directory.join(format!("font-{}.ttf", uuid::Uuid::new_v4()));
            std::fs::write(&temporary, bytes).map_err(|e| e.to_string())?;
            std::fs::rename(&temporary, &font).map_err(|e| e.to_string())?;
        }
        // A private alias distinguishes the bundled bytes from a system face.
        let (comparison, file) = font
            .to_str()
            .map(|path| ("eq", path))
            .unwrap_or(("contains", filename.as_str()));
        let file = glib::markup_escape_text(file);
        rules.push_str(&format!(
            r#"<match target="scan"><test name="file" compare="{comparison}"><string>{file}</string></test><edit name="family" mode="prepend"><string>{alias}</string></edit></match>"#
        ));
        files.push(CString::new(font.as_os_str().as_bytes()).map_err(|e| e.to_string())?);
    }
    rules.push_str("</fontconfig>");
    let rules = CString::new(rules).map_err(|e| e.to_string())?;
    let registered = unsafe {
        let config = FcConfigGetCurrent();
        !config.is_null()
            && FcConfigParseAndLoadFromMemory(config, rules.as_ptr(), 1) != 0
            && files
                .iter()
                .all(|file| FcConfigAppFontAddFile(config, file.as_ptr()) != 0)
    };
    if !registered {
        return Err("GTK could not load the bundled source fonts.".into());
    }
    Ok(())
}

pub fn is_private_family(name: &str) -> bool {
    FACES.iter().any(|&(_, alias, _)| alias == name)
}

pub fn source_family_css(name: &str) -> String {
    match name.to_ascii_lowercase().as_str() {
        "system ui" | "sans serif" => return "sans-serif".into(),
        "system mono" => return "monospace".into(),
        "serif" => return "serif".into(),
        _ => {}
    }
    if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        return "monospace".into();
    }
    let family = FACES
        .iter()
        .find(|&&(family, _, _)| family.eq_ignore_ascii_case(name))
        .map_or(name, |&(_, alias, _)| alias);
    format!(r#""{}""#, family.replace('\\', "\\\\").replace('"', "\\\""))
}
