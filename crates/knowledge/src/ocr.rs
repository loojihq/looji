//! Text from pictures and from scanned PDFs, so a screenshot of an error, a photographed whiteboard, or a scan of a
//! signed form can be searched. On Windows, the OCR and the PDF support that ship with the system do the work, so
//! nothing is downloaded. Elsewhere the `pdftoppm` and `tesseract` commands are used when they are installed. Without
//! them a picture or a scanned PDF is reported as having no text, rather than indexed as nothing.

/// The most text kept from one picture or scanned PDF, in characters.
const MAX_CHARS: usize = 20_000;
/// Pictures larger than this are not read.
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// The most pages of a scanned PDF that are read. Each page takes a moment, so a long scan is cut short.
pub const MAX_OCR_PAGES: usize = 30;
/// How wide a page is drawn for reading, in pixels. Wide enough that small type is legible to the OCR.
#[cfg(windows)]
const PAGE_WIDTH: u32 = 1600;

/// The words in a picture (PNG, JPEG, BMP, GIF), or why none could be read.
pub fn text(bytes: &[u8]) -> Result<String, String> {
    let read = std::panic::catch_unwind(|| read_image(bytes));
    let found = match read {
        Ok(result) => result?,
        Err(_) => return Err("the picture could not be read".to_string()),
    };
    let found = found
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if found.chars().count() < 3 {
        return Err("no text was found in the picture".to_string());
    }
    Ok(found.chars().take(MAX_CHARS).collect())
}

/// The text of each page of a scanned PDF that has any, with its page number. Only the first `MAX_OCR_PAGES` pages
/// are read.
pub fn pdf_pages(bytes: &[u8]) -> Result<Vec<(u32, String)>, String> {
    let images = match std::panic::catch_unwind(|| render_pages(bytes)) {
        Ok(result) => result?,
        Err(_) => return Err("the pages of the PDF could not be drawn".to_string()),
    };
    let mut found = Vec::new();
    let mut chars = 0usize;
    for (index, image) in images.iter().enumerate() {
        let Ok(words) = text(image) else {
            continue;
        };
        chars += words.chars().count();
        found.push((index as u32 + 1, words));
        if chars >= MAX_CHARS {
            break;
        }
    }
    if found.is_empty() {
        return Err("OCR found no words on its pages".to_string());
    }
    Ok(found)
}

/// Windows' runtime is started on this thread for as long as the guard lives, and stopped when it is dropped, so the
/// PDF and OCR objects made inside are released before the thread moves on. It is single-threaded because the PDF
/// renderer crashed the process as it exited when the runtime was multi-threaded.
#[cfg(windows)]
struct Runtime(bool);

#[cfg(windows)]
impl Runtime {
    fn start() -> Self {
        use windows::Win32::System::WinRT::{RO_INIT_SINGLETHREADED, RoInitialize};
        // A thread that is already set up in another way refuses, and then there is nothing to stop.
        let started = unsafe { RoInitialize(RO_INIT_SINGLETHREADED) }.is_ok();
        Self(started)
    }
}

#[cfg(windows)]
impl Drop for Runtime {
    fn drop(&mut self) {
        use windows::Win32::System::WinRT::RoUninitialize;
        if self.0 {
            unsafe { RoUninitialize() };
        }
    }
}

/// The bytes in a WinRT stream, positioned at the start.
#[cfg(windows)]
fn stream_of(
    bytes: &[u8],
) -> Result<windows::Storage::Streams::InMemoryRandomAccessStream, String> {
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
    let fail = |err: windows::core::Error| format!("the file could not be read: {err}");
    let stream = InMemoryRandomAccessStream::new().map_err(fail)?;
    let writer = DataWriter::CreateDataWriter(&stream).map_err(fail)?;
    writer.WriteBytes(bytes).map_err(fail)?;
    writer.StoreAsync().map_err(fail)?.join().map_err(fail)?;
    writer.DetachStream().map_err(fail)?;
    stream.Seek(0).map_err(fail)?;
    Ok(stream)
}

/// Everything written to a WinRT stream, as bytes.
#[cfg(windows)]
fn read_stream(
    stream: &windows::Storage::Streams::InMemoryRandomAccessStream,
) -> Result<Vec<u8>, String> {
    use windows::Storage::Streams::DataReader;
    let fail = |err: windows::core::Error| format!("the page could not be read: {err}");
    let size = stream.Size().map_err(fail)? as u32;
    if size == 0 {
        return Ok(Vec::new());
    }
    stream.Seek(0).map_err(fail)?;
    let reader = DataReader::CreateDataReader(stream).map_err(fail)?;
    reader.LoadAsync(size).map_err(fail)?.join().map_err(fail)?;
    let mut out = vec![0u8; size as usize];
    reader.ReadBytes(&mut out).map_err(fail)?;
    Ok(out)
}

#[cfg(windows)]
fn read_image(bytes: &[u8]) -> Result<String, String> {
    use windows::{Graphics::Imaging::BitmapDecoder, Media::Ocr::OcrEngine};
    let fail = |err: windows::core::Error| format!("the picture could not be read: {err}");
    let _runtime = Runtime::start();
    let stream = stream_of(bytes)?;
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

/// Each page of a PDF drawn as a PNG picture, up to `MAX_OCR_PAGES`.
#[cfg(windows)]
fn render_pages(bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    use windows::{
        Data::Pdf::{PdfDocument, PdfPageRenderOptions},
        Storage::Streams::InMemoryRandomAccessStream,
    };
    let fail = |err: windows::core::Error| format!("the PDF could not be drawn: {err}");
    let _runtime = Runtime::start();
    let source = stream_of(bytes)?;
    let document = PdfDocument::LoadFromStreamAsync(&source)
        .map_err(fail)?
        .join()
        .map_err(|_| "the PDF could not be opened to draw its pages".to_string())?;
    let count = document
        .PageCount()
        .map_err(fail)?
        .min(MAX_OCR_PAGES as u32);
    let options = PdfPageRenderOptions::new().map_err(fail)?;
    options.SetDestinationWidth(PAGE_WIDTH).map_err(fail)?;
    let mut pages = Vec::new();
    for index in 0..count {
        let page = document.GetPage(index).map_err(fail)?;
        let out = InMemoryRandomAccessStream::new().map_err(fail)?;
        page.RenderWithOptionsToStreamAsync(&out, &options)
            .map_err(fail)?
            .join()
            .map_err(fail)?;
        pages.push(read_stream(&out)?);
    }
    Ok(pages)
}

/// A folder for the files an outside command reads and writes. It is deleted when dropped.
#[cfg(not(windows))]
struct Scratch(std::path::PathBuf);

#[cfg(not(windows))]
impl Scratch {
    fn new() -> Result<Self, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or_default();
        let dir = std::env::temp_dir().join(format!("looji-ocr-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir)
            .map_err(|err| format!("a scratch folder could not be made: {err}"))?;
        Ok(Self(dir))
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.0.join(name)
    }
}

#[cfg(not(windows))]
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Runs an outside command and returns what it printed. A command that isn't installed is said so plainly.
#[cfg(not(windows))]
fn run(program: &str, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>, String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => format!("{program} is not installed on this computer"),
            _ => format!("{program} could not run: {err}"),
        })?;
    if !output.status.success() {
        return Err(format!(
            "{program} could not read it: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.stdout)
}

#[cfg(not(windows))]
fn read_image(bytes: &[u8]) -> Result<String, String> {
    use std::ffi::OsStr;
    let scratch = Scratch::new()?;
    let file = scratch.path("picture");
    std::fs::write(&file, bytes)
        .map_err(|err| format!("the picture could not be copied: {err}"))?;
    let words = run("tesseract", &[file.as_os_str(), OsStr::new("stdout")])?;
    Ok(String::from_utf8_lossy(&words).into_owned())
}

#[cfg(not(windows))]
fn render_pages(bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    use std::ffi::OsStr;
    let scratch = Scratch::new()?;
    let pdf = scratch.path("scan.pdf");
    std::fs::write(&pdf, bytes).map_err(|err| format!("the PDF could not be copied: {err}"))?;
    let last = MAX_OCR_PAGES.to_string();
    let prefix = scratch.path("page");
    run(
        "pdftoppm",
        &[
            OsStr::new("-png"),
            OsStr::new("-r"),
            OsStr::new("200"),
            OsStr::new("-l"),
            OsStr::new(last.as_str()),
            pdf.as_os_str(),
            prefix.as_os_str(),
        ],
    )?;
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&scratch.0)
        .map_err(|err| format!("the pages could not be listed: {err}"))?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("page-"))
        })
        .collect();
    // pdftoppm pads page numbers to the same width, so sorting by name keeps the pages in order.
    files.sort();
    files
        .iter()
        .map(|file| std::fs::read(file).map_err(|err| format!("a page could not be read: {err}")))
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn something_that_is_not_a_picture_or_a_pdf_is_an_error_not_a_panic() {
        assert!(super::text(b"definitely not an image").is_err());
        assert!(super::text(&[]).is_err());
        assert!(super::pdf_pages(b"definitely not a pdf").is_err());
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

    /// Reads the scanned PDF named by `LOOJI_SCAN` and prints each page's text:
    /// `LOOJI_SCAN=scan.pdf cargo test -p looji-knowledge real_scan -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a scanned PDF, named by LOOJI_SCAN"]
    fn real_scan() {
        let path = std::env::var("LOOJI_SCAN").expect("set LOOJI_SCAN to a PDF");
        match super::pdf_pages(&std::fs::read(path).unwrap()) {
            Ok(pages) => {
                for (number, text) in pages {
                    println!("--- page {number}\n{text}");
                }
            }
            Err(reason) => println!("not indexed: {reason}"),
        }
    }
}
