fn main() {
    let old = "<html><body><p> </p></body></html>";
    let new = "<html><body><p> </p><p class=\"a\"> </p></body></html>";

    let patches = hotmeal::diff::diff_html(old, new);
    println!("Patches ({:?}):", patches.as_ref().map(|p| p.len()));
    if let Ok(patches) = patches {
        for patch in &patches {
            println!("  {:?}", patch);
        }
    }
}
