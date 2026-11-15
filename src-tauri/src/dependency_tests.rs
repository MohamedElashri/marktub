//! Runtime regressions for security fixes in the native dependency stack.
use glib::variant::ToVariant;

#[test]
fn glib_string_variant_iteration_is_sound_under_optimization() {
    let values = ["first", "العربية", "文", "last"];
    let variant = values.to_variant();
    assert_eq!(
        variant.array_iter_str().unwrap().collect::<Vec<_>>(),
        values
    );
    assert_eq!(
        variant.array_iter_str().unwrap().rev().collect::<Vec<_>>(),
        values.into_iter().rev().collect::<Vec<_>>()
    );
    assert_eq!(variant.array_iter_str().unwrap().last(), Some("last"));
    assert_eq!(variant.array_iter_str().unwrap().nth(1), Some("العربية"));
    assert_eq!(variant.array_iter_str().unwrap().nth_back(1), Some("文"));
    let mut mixed = variant.array_iter_str().unwrap();
    assert_eq!(mixed.next(), Some("first"));
    assert_eq!(mixed.next_back(), Some("last"));
    assert_eq!(mixed.collect::<Vec<_>>(), ["العربية", "文"]);
}
