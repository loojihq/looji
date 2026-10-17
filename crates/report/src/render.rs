//! Draws laid-out pages as a PDF with the built-in Helvetica faces. The layout has already decided where everything
//! goes, so this only translates it: y is measured from the bottom of the page in PDF, and from the top in layout.

use printpdf::{
    BuiltinFont, Color, IndirectFontRef, Line, Mm, PdfDocument, PdfLayerReference, Point, Rgb,
};

use crate::{
    design::{PAGE_HEIGHT_MM, PAGE_WIDTH_MM, Rgb8},
    layout::{Op, Page},
    text::Face,
};

pub struct Fonts {
    regular: IndirectFontRef,
    bold: IndirectFontRef,
    italic: IndirectFontRef,
}

impl Fonts {
    fn pick(&self, face: Face) -> &IndirectFontRef {
        match face {
            Face::Regular => &self.regular,
            Face::Bold => &self.bold,
            Face::Italic => &self.italic,
        }
    }
}

pub fn colour(rgb: Rgb8) -> Color {
    let channel = |value: u8| f32::from(value) / 255.0;
    Color::Rgb(Rgb::new(
        channel(rgb.0),
        channel(rgb.1),
        channel(rgb.2),
        None,
    ))
}

/// Writes every page into one document. `title` is the document's title, shown by the viewer.
pub fn draw_document(title: &str, pages: &[Page]) -> Result<Vec<u8>, printpdf::Error> {
    let (document, first_page, first_layer) =
        PdfDocument::new(title, Mm(PAGE_WIDTH_MM), Mm(PAGE_HEIGHT_MM), "Page 1");
    let fonts = Fonts {
        regular: document.add_builtin_font(BuiltinFont::Helvetica)?,
        bold: document.add_builtin_font(BuiltinFont::HelveticaBold)?,
        italic: document.add_builtin_font(BuiltinFont::HelveticaOblique)?,
    };
    for (index, page) in pages.iter().enumerate() {
        let layer = if index == 0 {
            document.get_page(first_page).get_layer(first_layer)
        } else {
            let (page_index, layer_index) = document.add_page(
                Mm(PAGE_WIDTH_MM),
                Mm(PAGE_HEIGHT_MM),
                format!("Page {}", index + 1),
            );
            document.get_page(page_index).get_layer(layer_index)
        };
        draw_page(&layer, page, &fonts);
    }
    document.save_to_bytes()
}

fn draw_page(layer: &PdfLayerReference, page: &Page, fonts: &Fonts) {
    for op in &page.ops {
        match op {
            Op::Text {
                x,
                baseline,
                text,
                size_pt,
                face,
                colour: text_colour,
            } => {
                layer.set_fill_color(colour(*text_colour));
                layer.use_text(
                    text.as_str(),
                    *size_pt,
                    Mm(*x),
                    Mm(PAGE_HEIGHT_MM - baseline),
                    fonts.pick(*face),
                );
            }
            Op::Rule {
                x_start,
                x_end,
                y,
                thickness_pt,
                colour: rule_colour,
            } => {
                layer.set_outline_color(colour(*rule_colour));
                layer.set_outline_thickness(*thickness_pt);
                let height = PAGE_HEIGHT_MM - y;
                layer.add_line(Line {
                    points: vec![
                        (Point::new(Mm(*x_start), Mm(height)), false),
                        (Point::new(Mm(*x_end), Mm(height)), false),
                    ],
                    is_closed: false,
                });
            }
        }
    }
}
