#![allow(dead_code)]
pub fn hex(s: &str) -> Vec<u8> {
    s.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
}

pub fn fixture(name: &str) -> Vec<u8> {
    let text = include_str!("fixtures/frames.txt");
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix(name) {
            if rest.starts_with(' ') {
                return hex(rest);
            }
        }
    }
    panic!("no fixture {name}");
}
