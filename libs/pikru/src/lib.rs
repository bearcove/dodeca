/// Render pikchr source to SVG.
///
/// Returns the SVG string on success, or an error with diagnostics.
pub fn pikchr(source: &str) -> Result<String, miette::Report> {
    // TODO: implement the actual parser and renderer
    let _ = source;
    Err(miette::miette!("not yet implemented"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_test() {
        let result = pikchr(r#"box "Hello""#);
        // For now, just check it returns *something* (will fail until implemented)
        assert!(result.is_ok(), "pikchr failed: {:?}", result.err());
    }
}
