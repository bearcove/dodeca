use thiserror::Error;

#[derive(Debug, Error)]
pub enum SavageError {
    #[error("XML parsing error: {0}")]
    XmlParse(#[from] quick_xml::Error),

    #[error("Invalid SVG: {0}")]
    InvalidSvg(String),

    #[error("Invalid path data: {0}")]
    InvalidPath(String),

    #[error("UTF-8 error: {0}")]
    Utf8(#[from] std::str::Utf8Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
