//! Text from pictures, so a screenshot of an error or a photographed whiteboard can be searched. It uses the OCR that
//! Windows provides, which runs on the machine and needs no download. Elsewhere there is no reader yet, and a picture
//! is reported as having none rather than indexed as nothing.

/// The most text kept from one picture, in characters.
const MAX_CHARS: usize = 20_000;
/// Pictures larger than this are not read.
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;

/// The words in a picture (PNG, JPEG, BMP, GIF), or why none could be read.
pub fn text(bytes: &[u8]) -> Result<String, String> {
    let read = std::panic::catch_unwind(|| read(bytes));
    let text = match read {
        Ok(result) => result?,
        Err(_) => return Err("the picture could not be read".to_string()),
    };
    let text: String = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if text.chars().count() < 3 {
        return Err("no text was found in the picture".to_string());
    }
    Ok(text.chars().take(MAX_CHARS).collect())
}

#[cfg(windows)]
fn read(bytes: &[u8]) -> Result<String, String> {
    use windows::{
        Graphics::Imaging::BitmapDecoder,
        Media::Ocr::OcrEngine,
        Storage::Streams::{DataWriter, InMemoryRandomAccessStream},
        Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize},
    };

    let fail = |err: windows::core::Error| format!("the picture could not be read: {err}");
    // Already initialised on a thread that has been used before, which is fine.
    unsafe {
        let _ = RoInitialize(RO_INIT_MULTITHREADED);
    }
    let stream = InMemoryRandomAccessStream::new().map_err(fail)?;
    let writer = DataWriter::CreateDataWriter(&stream).map_err(fail)?;
    writer.WriteBytes(bytes).map_err(fail)?;
    writer.StoreAsync().map_err(fail)?.join().map_err(fail)?;
    writer.DetachStream().map_err(fail)?;
    stream.Seek(0).map_err(fail)?;
    let decoder = BitmapDecoder::CreateAsync(&stream)
        .map_err(fail)?
        .join()
        .map_err(|_| "not a picture Windows can read".to_string())?;
    let bitmap = decoder
        .GetSoftwareBitmapAsync()
        .map_err(fail)?
        .join()
        .map_err(fail)?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|_| "Windows has no OCR language installed".to_string())?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(fail)?
        .join()
        .map_err(fail)?;
    Ok(result.Text().map_err(fail)?.to_string())
}

#[cfg(not(windows))]
fn read(_bytes: &[u8]) -> Result<String, String> {
    Err("reading text from pictures is only available on Windows so far".to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn something_that_is_not_a_picture_is_an_error_not_a_panic() {
        assert!(super::text(b"definitely not an image").is_err());
        assert!(super::text(&[]).is_err());
    }

    /// Reads the picture named by `LOOJI_IMAGE` and prints its text:
    /// `LOOJI_IMAGE=shot.png cargo test -p looji-knowledge real_image -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a picture, named by LOOJI_IMAGE"]
    fn real_image() {
        let path = std::env::var("LOOJI_IMAGE").expect("set LOOJI_IMAGE to a picture");
        match super::text(&std::fs::read(path).unwrap()) {
            Ok(text) => println!("{text}"),
            Err(reason) => println!("not indexed: {reason}"),
        }
    }
}
