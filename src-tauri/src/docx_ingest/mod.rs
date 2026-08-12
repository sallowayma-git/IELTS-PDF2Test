pub(crate) mod drawings;
pub(crate) mod model;
pub(crate) mod numbering;
mod package;
pub(crate) mod paragraphs;
pub(crate) mod render_fallback;
pub(crate) mod sections;
pub(crate) mod smartart;
pub(crate) mod styles;
pub(crate) mod tables;
pub(crate) mod text_boxes;
pub(crate) mod xml;

pub(crate) use package::{is_rejected_package_error, open_docx, DocxPackage, DocxPackageLimits};
