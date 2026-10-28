//! Text from PDF files, so a spec or a design brief kept in the repository can be searched like any document. Only the
//! text layer is read. A scanned PDF is pictures of pages, has no text layer, and is reported as such instead of
//! being indexed as nothing. PDFs come from anywhere, so reading one is cut short at a page limit and a size limit,
//! and a file that makes the reader panic is treated as unreadable.

/// The most pages read from one file.
const MAX_PAGES: usize = 200;
/// The most text kept from one file, in characters.
const MAX_CHARS: usize = 400_000;

/// What reading a PDF's text layer found.
#[derive(Debug, PartialEq, Eq)]
pub enum Reading {
    /// The text of each page that has any, with its page number.
    Text(Vec<(u32, String)>),
    /// A PDF whose pages have no text layer, so they are probably scanned pictures.
    NoTextLayer,
    /// The bytes could not be read as a PDF, with the reason.
    Unreadable(String),
}

/// Reads the text layer of a PDF. A file that makes the reader panic is treated as unreadable.
pub fn read(bytes: &[u8]) -> Reading {
    match std::panic::catch_unwind(|| extract(bytes)) {
        Ok(Ok(pages)) if pages.is_empty() => Reading::NoTextLayer,
        Ok(Ok(pages)) => Reading::Text(pages),
        Ok(Err(reason)) => Reading::Unreadable(reason),
        Err(_) => Reading::Unreadable("the file could not be read as a PDF".to_string()),
    }
}

fn extract(bytes: &[u8]) -> Result<Vec<(u32, String)>, String> {
    let texts = pdf_extract::extract_text_from_mem_by_pages(bytes)
        .map_err(|err| format!("not readable as a PDF: {err}"))?;
    let mut found = Vec::new();
    let mut chars = 0usize;
    for (index, text) in texts.iter().enumerate().take(MAX_PAGES) {
        let text = tidy(text);
        if text.is_empty() {
            continue;
        }
        chars += text.chars().count();
        found.push((index as u32 + 1, text));
        if chars >= MAX_CHARS {
            break;
        }
    }
    Ok(found)
}

/// A page's text with the control characters and the runs of blank lines PDF extraction leaves taken out.
fn tidy(text: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in text.lines() {
        let line: String = line
            .chars()
            .filter(|c| !c.is_control() || *c == '\t')
            .collect();
        let line = line.trim_end();
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

#[cfg(test)]
pub(crate) mod fixtures {
    use lopdf::{
        Document, Object, Stream,
        content::{Content, Operation},
        dictionary,
    };

    /// A PDF with one page for each text given, in the standard Helvetica font.
    pub(crate) fn pdf(pages: &[&str]) -> Vec<u8> {
        let mut document = Document::with_version("1.5");
        let pages_id = document.new_object_id();
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = document.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let mut kids = Vec::new();
        for text in pages {
            let content = Content {
                operations: vec![
                    Operation::new("BT", vec![]),
                    Operation::new("Tf", vec!["F1".into(), 12.into()]),
                    Operation::new("Td", vec![72.into(), 720.into()]),
                    Operation::new("Tj", vec![Object::string_literal(*text)]),
                    Operation::new("ET", vec![]),
                ],
            };
            let content_id =
                document.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
            let page_id = document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content_id,
            });
            kids.push(page_id.into());
        }
        let count = kids.len() as i64;
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => count,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 595.into(), 842.into()],
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).unwrap();
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::{Reading, fixtures::pdf, read};

    #[test]
    fn the_text_of_each_page_is_read_with_its_number() {
        let bytes = pdf(&["Refund policy for customers", "Shipping times by region"]);
        let Reading::Text(pages) = read(&bytes) else {
            panic!("expected text");
        };
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].0, 1);
        assert!(pages[0].1.contains("Refund policy"), "{:?}", pages[0].1);
        assert_eq!(pages[1].0, 2);
        assert!(pages[1].1.contains("Shipping times"));
    }

    #[test]
    fn a_pdf_with_no_text_is_told_apart_from_garbage() {
        assert_eq!(read(&pdf(&[""])), Reading::NoTextLayer);
        assert!(
            matches!(read(b"not a pdf at all"), Reading::Unreadable(reason) if reason.contains("PDF"))
        );
    }
}

#[cfg(test)]
mod real_file {
    /// Reads the PDF named by `LOOJI_PDF` and prints what came out of it:
    /// `LOOJI_PDF=some.pdf cargo test -p looji-knowledge real_pdf -- --ignored --nocapture`.
    #[test]
    #[ignore = "needs a PDF, named by LOOJI_PDF"]
    fn real_pdf() {
        let path = std::env::var("LOOJI_PDF").expect("set LOOJI_PDF to a PDF file");
        let bytes = std::fs::read(path).unwrap();
        match super::read(&bytes) {
            super::Reading::Text(pages) => {
                for (number, text) in pages {
                    println!("--- page {number} ({} chars)\n{text}", text.chars().count());
                }
            }
            other => println!("no text layer, so the OCR path is used: {other:?}"),
        }
    }
}
