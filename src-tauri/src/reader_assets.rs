//! Native user-authorized raster reads and exclusive new-file HTML export.
use crate::{atomic_save, document::Provenance, files::FileError, scoped_file::ScopedFile};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{io::Read, path::Path};
fn dimensions(bytes: &[u8]) -> Option<(&'static str, u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.get(12..16) == Some(b"IHDR") {
        return Some((
            "image/png",
            u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?),
            u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?),
        ));
    }
    if bytes.starts_with(&[0xff, 0xd8]) {
        let mut at = 2;
        while at + 4 <= bytes.len() {
            if bytes[at] != 0xff {
                return None;
            }
            let code = bytes[at + 1];
            at += 2;
            if code == 0xd9 || code == 0xda {
                return None;
            }
            if code == 0x01 || (0xd0..=0xd8).contains(&code) {
                continue;
            }
            let size = u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as usize;
            if size < 2 || at + size > bytes.len() {
                return None;
            }
            if [0xc0, 0xc1, 0xc2].contains(&code) {
                let height = u16::from_be_bytes(bytes.get(at + 3..at + 5)?.try_into().ok()?) as u32;
                let width = u16::from_be_bytes(bytes.get(at + 5..at + 7)?.try_into().ok()?) as u32;
                return Some(("image/jpeg", width, height));
            }
            at += size;
        }
    }
    None
}
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Image {
    pub data_url: String,
    pub width: u32,
    pub height: u32,
}
pub fn image(path: &Path) -> Result<Image, FileError> {
    let canonical = std::fs::canonicalize(path)?;
    let scoped = ScopedFile::selected(&canonical)?.authorize_selection(path);
    image_scoped(scoped)
}
pub(crate) fn example_image(
    document: &Path,
    resources: &Path,
    link: &str,
) -> Result<Option<Image>, FileError> {
    if link != "marktub-example.png"
        || std::fs::canonicalize(resources.join("guide/Marktub Guide.md"))
            .ok()
            .as_deref()
            != Some(document)
    {
        return Ok(None);
    }
    image(&resources.join("guide/marktub-example.png")).map(Some)
}
pub(crate) fn image_scoped(scoped: ScopedFile) -> Result<Image, FileError> {
    let (mut file, _, _) = scoped.snapshot_asset(8 * 1024 * 1024)?;
    let mut bytes = vec![];
    file.read_to_end(&mut bytes)?;
    image_bytes(&bytes)
}
pub(crate) fn image_bytes(bytes: &[u8]) -> Result<Image, FileError> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err(FileError::new("image", "Raster exceeds 8 MiB."));
    }
    let (mime, width, height) = dimensions(bytes).ok_or_else(|| {
        FileError::new(
            "image",
            "Only valid PNG and JPEG headers are supported; SVG and remote images are source-only.",
        )
    })?;
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || width as u64 * height as u64 > 16_000_000
    {
        return Err(FileError::new(
            "image",
            "Image exceeds the 4,096-pixel side / 16-megapixel decode budget.",
        ));
    }
    Ok(Image {
        data_url: format!("data:{mime};base64,{}", STANDARD.encode(bytes)),
        width,
        height,
    })
}
pub fn export(
    path: &Path,
    content: &str,
    protected: &[std::path::PathBuf],
) -> Result<(), FileError> {
    let target = ScopedFile::destination(path)?;
    if protected
        .iter()
        .any(|root| target.canonical_path().starts_with(root))
    {
        return Err(FileError::new(
            "private-storage",
            "HTML export cannot write to application storage.",
        ));
    }
    let document = Provenance::new(content.as_bytes()).map_err(|_| {
        FileError::new(
            "export",
            "HTML export exceeds document encoding or byte limits.",
        )
    })?;
    atomic_save::save_as(&target, &document.snapshot())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn example_image_requires_the_exact_bundled_document_and_reference() {
        let dir = tempfile::tempdir().unwrap();
        let resources = dir.path().join("resources");
        let fixtures = resources.join("guide");
        std::fs::create_dir_all(&fixtures).unwrap();
        let guide = fixtures.join("Marktub Guide.md");
        std::fs::write(&guide, b"# Guide").unwrap();
        std::fs::write(
            fixtures.join("marktub-example.png"),
            include_bytes!("../icons/256x256.png"),
        )
        .unwrap();
        let guide = std::fs::canonicalize(guide).unwrap();
        assert_eq!(
            example_image(&guide, &resources, "marktub-example.png")
                .unwrap()
                .unwrap()
                .width,
            256
        );
        for link in [
            "../marktub-example.png",
            "file:///marktub-example.png",
            "https://example.com/image.png",
        ] {
            assert!(example_image(&guide, &resources, link).unwrap().is_none());
        }
        let lookalike = dir.path().join("Marktub Guide.md");
        std::fs::write(&lookalike, b"# Guide").unwrap();
        assert!(
            example_image(
                &std::fs::canonicalize(lookalike).unwrap(),
                &resources,
                "marktub-example.png"
            )
            .unwrap()
            .is_none()
        );
    }
    #[test]
    fn raster_headers_and_exclusive_export() {
        let dir = tempfile::tempdir().unwrap();
        let image_path = dir.path().join("img");
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(32u32.to_be_bytes());
        png.extend(16u32.to_be_bytes());
        std::fs::write(&image_path, &png).unwrap();
        assert!(
            image(&image_path)
                .unwrap()
                .data_url
                .starts_with("data:image/png;base64,")
        );
        png[16..20].copy_from_slice(&99999u32.to_be_bytes());
        std::fs::write(&image_path, png).unwrap();
        assert_eq!(image(&image_path).unwrap_err().code, "image");
        std::fs::write(&image_path, b"<svg onload='run()'/>").unwrap();
        assert!(image(&image_path).is_err());
        let output = dir.path().join("doc.html");
        export(&output, "<!doctype html><p>saved</p>", &[]).unwrap();
        assert!(export(&output, "replace", &[]).is_err());
        assert!(std::fs::read_to_string(output).unwrap().contains("saved"));
    }
}
