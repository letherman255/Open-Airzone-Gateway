use airzone_core::ws::*;

#[test]
fn ws_text_frame_lengths() {
    assert_eq!(&text_frame("hi")[..2], &[0x81, 2]);
    let long = "x".repeat(300);
    let f = text_frame(&long);
    assert_eq!(&f[..4], &[0x81, 126, 1, 44]);
    assert_eq!(f.len(), 304);
}

#[test]
fn ws_client_frame_unmasks() {
    // RFC 6455 5.7: masked "Hello"
    let f = [0x81, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58];
    assert_eq!(client_frame(&f), Some((1, b"Hello".to_vec(), 11)));
    assert_eq!(client_frame(&f[..5]), None);
}
