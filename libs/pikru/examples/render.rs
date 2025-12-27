fn main() { 
    let source = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let result = pikru::pikchr(&source).unwrap();
    println!("{}", result);
}
