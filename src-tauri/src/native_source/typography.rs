//! Apply reading metrics to Arabic paragraphs in the plain GTK editor.
use crate::fonts::native::ARABIC_FAMILY;
use gtk::glib::{Unichar, UnicodeScript};
use gtk::prelude::*;
use std::{cell::Cell, rc::Rc};

pub fn arabic_paragraphs(buffer: &gtk::TextBuffer) -> gtk::TextTag {
    let tag = gtk::TextTag::builder().family(ARABIC_FAMILY).build();
    buffer
        .tag_table()
        .expect("GtkTextBuffer has a tag table")
        .add(&tag);
    style_paragraphs(buffer, &tag, 0, buffer.char_count());

    // Retag the resulting paragraphs after inserts and joins. Keep the text
    // and offsets in GTK; tags do not enter the shared editing transaction.
    let touched = Rc::new(Cell::new(None));
    let pending = touched.clone();
    buffer.connect_insert_text(move |_, at, text| {
        let start = at.offset();
        pending.set(Some((start, start + text.chars().count() as i32)));
    });
    let pending = touched.clone();
    buffer.connect_delete_range(move |_, start, _| {
        pending.set(Some((start.offset(), start.offset())));
    });
    let paragraphs = tag.clone();
    buffer.connect_changed(move |buffer| {
        if let Some((start, end)) = touched.take() {
            style_paragraphs(buffer, &paragraphs, start, end);
        }
    });
    tag
}

fn style_paragraphs(buffer: &gtk::TextBuffer, tag: &gtk::TextTag, from: i32, to: i32) {
    let from = buffer.iter_at_offset(from).line();
    let to = buffer.iter_at_offset(to).line();
    for line in from..=to {
        let start = buffer.iter_at_line(line);
        let mut end = start;
        end.forward_line();
        if start == end {
            continue;
        }
        if start
            .text(&end)
            .is_some_and(|text| text.chars().any(|c| c.script() == UnicodeScript::Arabic))
        {
            buffer.apply_tag(tag, &start, &end);
        } else {
            buffer.remove_tag(tag, &start, &end);
        }
    }
}
