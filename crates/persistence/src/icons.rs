//! Workspace icons: validate an image, turn it into a square PNG, and store it in the icons directory.
//!
//! Nothing here touches the database. `SqliteStore::replace_workspace_icon` records the file name.

use std::{
    fs::{self, File},
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
};

use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, imageops::FilterType};
use looji_domain::WorkspaceId;
use thiserror::Error;
use uuid::Uuid;

/// Largest source file accepted, in bytes.
pub const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
/// Largest accepted width or height in pixels. Guards against decompression bombs.
pub const MAX_DIMENSION: u32 = 8000;
/// Side length of the stored icon. Big enough for 2x screens at sidebar size.
pub const ICON_SIZE: u32 = 256;

#[derive(Debug, Error)]
pub enum IconError {
    #[error("the image is larger than 10 MB")]
    TooLarge,
    #[error("the file is empty")]
    Empty,
    #[error("only PNG, JPEG and WebP images can be used as icons")]
    UnsupportedFormat,
    #[error("the image is {width}×{height} pixels, but each side can be at most {MAX_DIMENSION}")]
    TooLargeDimensions { width: u32, height: u32 },
    #[error("the image could not be read; it may be corrupt or use an unsupported variant ({0})")]
    Undecodable(String),
    #[error("the icon could not be saved: {0}")]
    Io(String),
    #[error("the workspace could not be updated: {0}")]
    Database(String),
    #[error("that workspace no longer exists")]
    WorkspaceNotFound,
}

/// The icons directory inside the app data directory.
pub fn icons_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("icons")
}

/// Full path of an icon file, given the name stored on the workspace.
pub fn icon_path(icons_dir: &Path, file_name: &str) -> PathBuf {
    icons_dir.join(file_name)
}

/// Deletes an icon file. A file that is already gone counts as deleted.
pub fn remove_icon(icons_dir: &Path, file_name: &str) -> io::Result<()> {
    match fs::remove_file(icon_path(icons_dir, file_name)) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Validates `source`, stores it as a square PNG in `icons_dir`, and returns the new file name.
///
/// The file name is unique per import, so a cached image can't be served for a replaced icon.
/// On error nothing is left behind in `icons_dir`.
pub fn import_icon(
    source: &Path,
    icons_dir: &Path,
    workspace: WorkspaceId,
) -> Result<String, IconError> {
    let bytes = read_capped(source)?;
    let format = detect_format(&bytes)?;

    // Header first: rejects huge images before any pixels are allocated.
    let (width, height) = ImageReader::with_format(Cursor::new(&bytes), format)
        .into_dimensions()
        .map_err(|err| IconError::Undecodable(err.to_string()))?;
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(IconError::TooLargeDimensions { width, height });
    }

    // Full decode: a truncated file can pass the header check and still fail here.
    let image = decode(&bytes, format)?;
    let icon = square_icon(image);
    let png = encode_png(&icon)?;
    write_icon(icons_dir, workspace, &png)
}

fn read_capped(source: &Path) -> Result<Vec<u8>, IconError> {
    let file = File::open(source).map_err(|err| IconError::Io(err.to_string()))?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| IconError::Io(err.to_string()))?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(IconError::TooLarge);
    }
    if bytes.is_empty() {
        return Err(IconError::Empty);
    }
    Ok(bytes)
}

/// Detects the format from magic bytes. The file extension is never trusted.
fn detect_format(bytes: &[u8]) -> Result<ImageFormat, IconError> {
    const PNG_SIGNATURE: &[u8] = &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
    if bytes.starts_with(PNG_SIGNATURE) {
        Ok(ImageFormat::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Ok(ImageFormat::Jpeg)
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Ok(ImageFormat::WebP)
    } else {
        Err(IconError::UnsupportedFormat)
    }
}

/// Decodes the image and applies its EXIF orientation, so phone photos aren't sideways.
fn decode(bytes: &[u8], format: ImageFormat) -> Result<DynamicImage, IconError> {
    let undecodable = |err: image::ImageError| IconError::Undecodable(err.to_string());
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), format)
        .into_decoder()
        .map_err(undecodable)?;
    let orientation = decoder.orientation().map_err(undecodable)?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(undecodable)?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// Center-crops to a square and scales to `ICON_SIZE`.
fn square_icon(image: DynamicImage) -> DynamicImage {
    let side = image.width().min(image.height());
    let x = (image.width() - side) / 2;
    let y = (image.height() - side) / 2;
    image
        .crop_imm(x, y, side, side)
        .resize_exact(ICON_SIZE, ICON_SIZE, FilterType::Lanczos3)
}

fn encode_png(icon: &DynamicImage) -> Result<Vec<u8>, IconError> {
    let mut encoded = Cursor::new(Vec::new());
    icon.write_to(&mut encoded, ImageFormat::Png)
        .map_err(|err| IconError::Io(err.to_string()))?;
    Ok(encoded.into_inner())
}

/// Writes to a temporary name, then renames into place, so a crash never leaves a partial icon.
fn write_icon(icons_dir: &Path, workspace: WorkspaceId, png: &[u8]) -> Result<String, IconError> {
    fs::create_dir_all(icons_dir).map_err(|err| IconError::Io(err.to_string()))?;
    let unique = Uuid::new_v4();
    let file_name = format!("workspace-{}-{unique}.png", workspace.as_uuid());
    let temp = icons_dir.join(format!(".{unique}.tmp"));

    let result =
        fs::write(&temp, png).and_then(|()| fs::rename(&temp, icon_path(icons_dir, &file_name)));
    match result {
        Ok(()) => Ok(file_name),
        Err(err) => {
            let _ = fs::remove_file(&temp);
            Err(IconError::Io(err.to_string()))
        }
    }
}
