fn main() {
    let old = "<html><body><p> </p></body></html>";
    let new = "<html><body><p> </p><p class=\"a\"> </p></body></html>";

    let edit_ops =
        facet_diff::tree_diff(&hotmeal::parse_untyped(old), &hotmeal::parse_untyped(new));
    println!("Edit ops ({}):", edit_ops.len());
    for op in &edit_ops {
        println!("  {:?}", op);
    }

    let patches = hotmeal::diff::diff_html(old, new);
    println!("\nPatches: {:?}", patches);
}
