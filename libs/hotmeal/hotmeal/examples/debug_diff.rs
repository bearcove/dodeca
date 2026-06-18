fn main() {
    use hotmeal::StrTendril;

    let old = StrTendril::from("<html><body><p> </p></body></html>");
    let new = StrTendril::from("<html><body><p> </p><p class=\"a\"> </p></body></html>");

    let patches = hotmeal::diff_html(&old, &new);
    println!("Patches ({:?}):", patches.as_ref().map(|p| p.len()));
    if let Ok(patches) = patches {
        for patch in &patches {
            println!("  {:?}", patch);
        }
    }
}
