//! Bundled and installed family names only; never a font path, download or file bridge.
use std::collections::BTreeSet;

#[cfg(target_os = "linux")]
pub(crate) mod native;

pub fn families() -> Vec<String> {
    let mut names = BTreeSet::new();
    platform(&mut names);
    #[cfg(target_os = "linux")]
    names.retain(|name| !native::is_private_family(name));
    names.into_iter().take(512).collect()
}
#[cfg(target_os = "macos")]
fn platform(names: &mut BTreeSet<String>) {
    use std::ffi::{c_char, c_void};
    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontManagerCopyAvailableFontFamilyNames() -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFArrayGetCount(a: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(a: *const c_void, i: isize) -> *const c_void;
        fn CFStringGetCString(s: *const c_void, b: *mut c_char, n: isize, e: u32) -> bool;
        fn CFRelease(a: *const c_void);
    }
    unsafe {
        let a = CTFontManagerCopyAvailableFontFamilyNames();
        if a.is_null() {
            return;
        }
        for i in 0..CFArrayGetCount(a).min(4096) {
            let s = CFArrayGetValueAtIndex(a, i);
            let mut buf = [0i8; 512];
            if !s.is_null()
                && CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, 0x08000100)
                && let Ok(s) = std::ffi::CStr::from_ptr(buf.as_ptr()).to_str()
                && !s.is_empty()
                && s.len() <= 128
                && !s.chars().any(char::is_control)
            {
                names.insert(s.into());
            }
        }
        CFRelease(a);
    }
}
#[cfg(target_os = "linux")]
fn platform(names: &mut BTreeSet<String>) {
    use std::ffi::{CStr, c_char, c_void};
    #[repr(C)]
    struct FontSet {
        nfont: i32,
        _capacity: i32,
        fonts: *mut *mut c_void,
    }
    #[link(name = "fontconfig")]
    unsafe extern "C" {
        fn FcPatternCreate() -> *mut c_void;
        fn FcPatternDestroy(p: *mut c_void);
        fn FcObjectSetCreate() -> *mut c_void;
        fn FcObjectSetAdd(o: *mut c_void, s: *const c_char) -> i32;
        fn FcObjectSetDestroy(o: *mut c_void);
        fn FcFontList(c: *mut c_void, p: *mut c_void, o: *mut c_void) -> *mut FontSet;
        fn FcFontSetDestroy(s: *mut FontSet);
        fn FcPatternGetString(p: *mut c_void, n: *const c_char, i: i32, s: *mut *mut u8) -> i32;
    }
    unsafe {
        let p = FcPatternCreate();
        let o = FcObjectSetCreate();
        if p.is_null() || o.is_null() {
            if !p.is_null() {
                FcPatternDestroy(p);
            }
            if !o.is_null() {
                FcObjectSetDestroy(o);
            }
            return;
        }
        FcObjectSetAdd(o, c"family".as_ptr());
        let set = FcFontList(std::ptr::null_mut(), p, o);
        if !set.is_null() {
            for i in 0..(*set).nfont.clamp(0, 8192) {
                let mut s = std::ptr::null_mut();
                if FcPatternGetString(*(*set).fonts.add(i as usize), c"family".as_ptr(), 0, &mut s)
                    == 0
                    && !s.is_null()
                    && let Ok(s) = CStr::from_ptr(s.cast()).to_str()
                    && !s.is_empty()
                    && s.len() <= 128
                    && !s.chars().any(char::is_control)
                {
                    names.insert(s.into());
                }
            }
            FcFontSetDestroy(set);
        }
        FcObjectSetDestroy(o);
        FcPatternDestroy(p);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_font_names_are_bounded_and_unique() {
        let names = super::families();
        assert!(names.len() <= 512);
        assert!(names.windows(2).all(|w| w[0] < w[1]));
        assert!(
            names
                .iter()
                .all(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
        );
    }
}
