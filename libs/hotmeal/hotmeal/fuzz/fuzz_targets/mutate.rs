#![no_main]

use libfuzzer_sys::fuzz_target;
use tendril::StrTendril;

static BASE_HTML: &str = include_str!("../corpus/mutate/dibs-homepage.html");

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    let mutated = apply_mutations(BASE_HTML, data);
    let tendril = StrTendril::from(mutated);
    let _ = hotmeal::parse(&tendril);
});

fn apply_mutations(base: &str, mutations: &[u8]) -> String {
    let base_bytes = base.as_bytes();
    let mut result = base_bytes.to_vec();

    let mut i = 0;
    while i + 2 < mutations.len() {
        let op = mutations[i] % 4;
        let pos_byte = mutations[i + 1];
        let val = mutations[i + 2];

        if result.is_empty() {
            break;
        }

        let pos = (pos_byte as usize * result.len()) / 256;
        let pos = pos.min(result.len().saturating_sub(1));

        match op {
            0 => {
                // Insert byte
                if result.len() < 1_000_000 {
                    result.insert(pos, val);
                }
            }
            1 => {
                // Delete byte
                if !result.is_empty() {
                    result.remove(pos);
                }
            }
            2 => {
                // Replace byte
                result[pos] = val;
            }
            3 => {
                // Insert interesting HTML chars
                let chars: &[u8] = match val % 8 {
                    0 => b"<",
                    1 => b">",
                    2 => b"</",
                    3 => b"/>",
                    4 => b"=\"",
                    5 => b"\"",
                    6 => b"&amp;",
                    7 => b"<!--",
                    _ => b"",
                };
                if result.len() + chars.len() < 1_000_000 {
                    for (j, &c) in chars.iter().enumerate() {
                        if pos + j <= result.len() {
                            result.insert(pos + j, c);
                        }
                    }
                }
            }
            _ => {}
        }

        i += 3;
    }

    String::from_utf8_lossy(&result).into_owned()
}
