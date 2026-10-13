use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

use image::{DynamicImage, GenericImageView, ImageFormat, Rgba, RgbaImage};
use looji_domain::{Workspace, WorkspaceId, WorkspaceRepository};
use looji_persistence::{
    Connection, SqliteStore,
    icons::{ICON_SIZE, IconError, icon_path, import_icon},
};
use tempfile::TempDir;

// ---- fixtures -------------------------------------------------------------

/// A 64x48 image split into a red left half and a blue right half.
fn split_image(width: u32, height: u32) -> DynamicImage {
    DynamicImage::ImageRgba8(RgbaImage::from_fn(width, height, |x, _| {
        if x < width / 2 {
            Rgba([230, 20, 20, 255])
        } else {
            Rgba([20, 20, 230, 255])
        }
    }))
}

fn encode(image: &DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, format).expect("fixture encodes");
    out.into_inner()
}

fn png() -> Vec<u8> {
    encode(&split_image(64, 48), ImageFormat::Png)
}

fn jpeg() -> Vec<u8> {
    encode(&split_image(64, 48), ImageFormat::Jpeg)
}

fn webp() -> Vec<u8> {
    // The bundled WebP encoder writes lossless files only.
    encode(&split_image(64, 48), ImageFormat::WebP)
}

/// Inserts an APP1/EXIF segment carrying only an Orientation tag after the JPEG SOI marker.
fn with_exif_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"MM\0*"); // big-endian, TIFF magic
    tiff.extend_from_slice(&8u32.to_be_bytes()); // first IFD at offset 8
    tiff.extend_from_slice(&1u16.to_be_bytes()); // one entry
    tiff.extend_from_slice(&0x0112u16.to_be_bytes()); // Orientation tag
    tiff.extend_from_slice(&3u16.to_be_bytes()); // SHORT
    tiff.extend_from_slice(&1u32.to_be_bytes()); // one value
    tiff.extend_from_slice(&orientation.to_be_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_be_bytes()); // no next IFD

    let mut payload = b"Exif\0\0".to_vec();
    payload.extend(tiff);
    let length = u16::try_from(payload.len() + 2).expect("small payload");

    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xFF, 0xE1]);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend(payload);
    out.extend_from_slice(&jpeg[2..]);
    out
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, bytes).expect("fixture writes");
    path
}

/// A source directory and an icons directory that doesn't exist yet.
fn dirs() -> (TempDir, PathBuf, PathBuf) {
    let temp = TempDir::new().unwrap();
    let source = temp.path().join("source");
    fs::create_dir_all(&source).unwrap();
    let icons = temp.path().join("data").join("icons");
    (temp, source, icons)
}

fn decode(path: &Path) -> DynamicImage {
    image::open(path).expect("output decodes")
}

// ---- valid inputs ---------------------------------------------------------

#[test]
fn imports_png_as_a_square_png() {
    let (_temp, source, icons) = dirs();
    let src = write(&source, "photo.png", &png());

    let name = import_icon(&src, &icons, WorkspaceId::new()).expect("PNG imports");

    let stored = icon_path(&icons, &name);
    let output = decode(&stored);
    assert_eq!(output.dimensions(), (ICON_SIZE, ICON_SIZE));
    assert_eq!(
        image::guess_format(&fs::read(&stored).unwrap()).unwrap(),
        ImageFormat::Png
    );
}

#[test]
fn imports_jpeg_as_a_square_png() {
    let (_temp, source, icons) = dirs();
    let src = write(&source, "photo.jpg", &jpeg());

    let name = import_icon(&src, &icons, WorkspaceId::new()).expect("JPEG imports");

    assert_eq!(
        decode(&icon_path(&icons, &name)).dimensions(),
        (ICON_SIZE, ICON_SIZE)
    );
}

#[test]
fn imports_webp_as_a_square_png() {
    let (_temp, source, icons) = dirs();
    let src = write(&source, "photo.webp", &webp());

    let name = import_icon(&src, &icons, WorkspaceId::new()).expect("WebP imports");

    assert_eq!(
        decode(&icon_path(&icons, &name)).dimensions(),
        (ICON_SIZE, ICON_SIZE)
    );
}

#[test]
fn center_crops_a_wide_image_before_scaling() {
    let (_temp, source, icons) = dirs();
    // Red on the left half, blue on the right. The output is square, so its edges show the
    // colours at the crop's sides. This checks the crop is centred and keeps the image's
    // colour layout; it doesn't prove the crop was square rather than a stretch.
    let src = write(
        &source,
        "wide.png",
        &encode(&split_image(200, 50), ImageFormat::Png),
    );

    let name = import_icon(&src, &icons, WorkspaceId::new()).unwrap();
    let output = decode(&icon_path(&icons, &name)).to_rgba8();

    let left = output.get_pixel(8, ICON_SIZE / 2);
    let right = output.get_pixel(ICON_SIZE - 8, ICON_SIZE / 2);
    assert!(left[0] > left[2], "left edge should be red, got {left:?}");
    assert!(
        right[2] > right[0],
        "right edge should be blue, got {right:?}"
    );
}

#[test]
fn applies_exif_orientation_so_rotated_photos_are_upright() {
    let (_temp, source, icons) = dirs();
    // Stored as 4 wide x 2 tall, red on the left. Orientation 6 means "rotate 90° clockwise to
    // display", which puts red on top and blue on the bottom.
    let stored = encode(&split_image(4, 2), ImageFormat::Jpeg);
    let src = write(&source, "rotated.jpg", &with_exif_orientation(&stored, 6));

    let name = import_icon(&src, &icons, WorkspaceId::new()).unwrap();
    let output = decode(&icon_path(&icons, &name)).to_rgba8();

    let top_left = output.get_pixel(ICON_SIZE / 4, ICON_SIZE / 4);
    let bottom_left = output.get_pixel(ICON_SIZE / 4, ICON_SIZE * 3 / 4);
    assert!(
        top_left[0] > top_left[2],
        "top should be red, got {top_left:?}"
    );
    assert!(
        bottom_left[2] > bottom_left[0],
        "bottom should be blue, got {bottom_left:?}"
    );
}

#[test]
fn transparency_survives_the_import() {
    let (_temp, source, icons) = dirs();
    let transparent = DynamicImage::ImageRgba8(RgbaImage::from_fn(32, 32, |x, y| {
        if x < 16 && y < 16 {
            Rgba([0, 0, 0, 0])
        } else {
            Rgba([200, 60, 60, 255])
        }
    }));
    let src = write(
        &source,
        "alpha.png",
        &encode(&transparent, ImageFormat::Png),
    );

    let name = import_icon(&src, &icons, WorkspaceId::new()).unwrap();
    let output = decode(&icon_path(&icons, &name)).to_rgba8();

    assert_eq!(
        output.get_pixel(4, 4)[3],
        0,
        "the transparent corner stays transparent"
    );
    assert_eq!(output.get_pixel(ICON_SIZE - 4, ICON_SIZE - 4)[3], 255);
}

#[test]
fn each_import_gets_a_new_file_name() {
    let (_temp, source, icons) = dirs();
    let src = write(&source, "photo.png", &png());
    let workspace = WorkspaceId::new();

    let first = import_icon(&src, &icons, workspace).unwrap();
    let second = import_icon(&src, &icons, workspace).unwrap();

    assert_ne!(
        first, second,
        "a replaced icon must not reuse the old file name"
    );
    assert!(!first.contains('/') && !first.contains('\\'));
}

// ---- rejected inputs ------------------------------------------------------

#[test]
fn a_truncated_file_fails_at_full_decode() {
    let (_temp, source, icons) = dirs();
    let full = png();
    let src = write(&source, "cut.png", &full[..full.len() / 2]);

    let result = import_icon(&src, &icons, WorkspaceId::new());

    assert!(
        matches!(result, Err(IconError::Undecodable(_))),
        "got {result:?}"
    );
}

#[test]
fn a_text_file_renamed_to_png_is_rejected_by_content() {
    let (_temp, source, icons) = dirs();
    let src = write(&source, "notes.png", b"just some text, not an image\n");

    let result = import_icon(&src, &icons, WorkspaceId::new());

    assert!(
        matches!(result, Err(IconError::UnsupportedFormat)),
        "got {result:?}"
    );
}

#[test]
fn a_zero_byte_file_is_rejected() {
    let (_temp, source, icons) = dirs();
    let src = write(&source, "empty.png", b"");

    let result = import_icon(&src, &icons, WorkspaceId::new());

    assert!(matches!(result, Err(IconError::Empty)), "got {result:?}");
}

#[test]
fn an_oversized_image_is_rejected_from_its_header() {
    let (_temp, source, icons) = dirs();
    let wide = encode(&split_image(8001, 1), ImageFormat::Png);
    let src = write(&source, "wide.png", &wide);

    let result = import_icon(&src, &icons, WorkspaceId::new());

    assert!(
        matches!(
            result,
            Err(IconError::TooLargeDimensions {
                width: 8001,
                height: 1
            })
        ),
        "got {result:?}"
    );
}

#[test]
fn a_failed_import_leaves_no_files_behind() {
    let (_temp, source, icons) = dirs();
    let src = write(&source, "bad.png", &png()[..40]);

    let _ = import_icon(&src, &icons, WorkspaceId::new());

    let leftovers = fs::read_dir(&icons)
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(leftovers, 0);
}

// ---- storing the icon on a workspace --------------------------------------

fn store_with_workspace(path: &Path) -> (SqliteStore, Workspace) {
    let store = SqliteStore::open(path).unwrap();
    let workspace = Workspace::new("Personal").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();
    (store, workspace)
}

#[test]
fn replacing_an_icon_records_it_and_deletes_the_previous_file() {
    let (temp, source, icons) = dirs();
    let db = temp.path().join("looji.db");
    let (store, workspace) = store_with_workspace(&db);
    let src = write(&source, "photo.png", &png());

    let first = import_icon(&src, &icons, workspace.id()).unwrap();
    store
        .replace_workspace_icon(&icons, workspace.id(), Some(&first))
        .unwrap();
    let second = import_icon(&src, &icons, workspace.id()).unwrap();
    store
        .replace_workspace_icon(&icons, workspace.id(), Some(&second))
        .unwrap();

    let loaded = WorkspaceRepository::find(&store, workspace.id())
        .unwrap()
        .unwrap();
    assert_eq!(loaded.icon_file(), Some(second.as_str()));
    assert!(
        !icon_path(&icons, &first).exists(),
        "previous icon should be deleted"
    );
    assert!(icon_path(&icons, &second).exists());
}

#[test]
fn clearing_an_icon_removes_its_file() {
    let (temp, source, icons) = dirs();
    let db = temp.path().join("looji.db");
    let (store, workspace) = store_with_workspace(&db);
    let src = write(&source, "photo.png", &png());
    let name = import_icon(&src, &icons, workspace.id()).unwrap();
    store
        .replace_workspace_icon(&icons, workspace.id(), Some(&name))
        .unwrap();

    store
        .replace_workspace_icon(&icons, workspace.id(), None)
        .unwrap();

    let loaded = WorkspaceRepository::find(&store, workspace.id())
        .unwrap()
        .unwrap();
    assert_eq!(loaded.icon_file(), None);
    assert!(!icon_path(&icons, &name).exists());
}

#[test]
fn a_forced_database_failure_rolls_back_and_leaves_no_orphan_file() {
    let (temp, source, icons) = dirs();
    let db = temp.path().join("looji.db");
    let (store, workspace) = store_with_workspace(&db);
    let src = write(&source, "photo.png", &png());

    // Keep a working icon in place first.
    let old = import_icon(&src, &icons, workspace.id()).unwrap();
    store
        .replace_workspace_icon(&icons, workspace.id(), Some(&old))
        .unwrap();

    // Make every icon write fail inside the database.
    let raw = Connection::open(&db).unwrap();
    raw.execute_batch(
        "CREATE TRIGGER fail_icon_write BEFORE UPDATE OF icon_file ON workspaces
         BEGIN SELECT RAISE(ABORT, 'forced failure'); END;",
    )
    .unwrap();
    drop(raw);

    let new = import_icon(&src, &icons, workspace.id()).unwrap();
    let result = store.replace_workspace_icon(&icons, workspace.id(), Some(&new));

    assert!(
        matches!(result, Err(IconError::Database(_))),
        "got {result:?}"
    );
    let loaded = WorkspaceRepository::find(&store, workspace.id())
        .unwrap()
        .unwrap();
    assert_eq!(
        loaded.icon_file(),
        Some(old.as_str()),
        "the old icon must still be recorded"
    );
    assert!(
        icon_path(&icons, &old).exists(),
        "the old icon file must still exist"
    );
    assert!(
        !icon_path(&icons, &new).exists(),
        "the new file must not be left behind"
    );
}

#[test]
fn an_icon_for_a_missing_workspace_leaves_no_file_behind() {
    let (temp, source, icons) = dirs();
    let db = temp.path().join("looji.db");
    let (store, _) = store_with_workspace(&db);
    let src = write(&source, "photo.png", &png());
    let missing = WorkspaceId::new();
    let name = import_icon(&src, &icons, missing).unwrap();

    let result = store.replace_workspace_icon(&icons, missing, Some(&name));

    assert!(
        matches!(result, Err(IconError::WorkspaceNotFound)),
        "got {result:?}"
    );
    assert!(!icon_path(&icons, &name).exists());
}

#[test]
fn a_rename_saved_from_a_stale_copy_keeps_the_current_icon() {
    let (temp, source, icons) = dirs();
    let db = temp.path().join("looji.db");
    let (store, mut workspace) = store_with_workspace(&db);
    let src = write(&source, "photo.png", &png());
    let name = import_icon(&src, &icons, workspace.id()).unwrap();
    store
        .replace_workspace_icon(&icons, workspace.id(), Some(&name))
        .unwrap();

    // `workspace` still has no icon, as if it were loaded before the icon was set.
    workspace.rename("Work").unwrap();
    WorkspaceRepository::save(&store, &workspace).unwrap();

    let loaded = WorkspaceRepository::find(&store, workspace.id())
        .unwrap()
        .unwrap();
    assert_eq!(loaded.name(), "Work");
    assert_eq!(loaded.icon_file(), Some(name.as_str()));
}
