#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;
use tendril::StrTendril;

#[derive(Arbitrary, Debug)]
struct Input {
    html_a: Vec<u8>,
    html_b: Vec<u8>,
}

fuzz_target!(|input: Input| {
    let a = String::from_utf8_lossy(&input.html_a);
    let b = String::from_utf8_lossy(&input.html_b);

    let a_tendril = StrTendril::from(a.as_ref());
    let b_tendril = StrTendril::from(b.as_ref());

    let doc_a = hotmeal::parse(&a_tendril);
    let doc_b = hotmeal::parse(&b_tendril);

    let patches = hotmeal::diff(&doc_a, &doc_b).expect("diff must always succeed");

    let mut patched = doc_a.clone();
    patched
        .apply_patches(patches)
        .expect("apply_patches must always succeed");

    // Compare body contents - the diff system only operates on body content,
    // so structural differences (one doc has body, other doesn't) won't be patched.
    // Both "no body" and "empty body" should produce empty body content.
    let patched_body = patched.to_body_html();
    let expected_body = doc_b.to_body_html();

    assert_eq!(
        patched_body, expected_body,
        "Patched body content should match target body content"
    );
});
