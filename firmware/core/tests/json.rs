use airzone_core::json::*;

#[test]
fn json_roundtrip() {
    let v = parse(r#"{"system":1,"zone":2,"value":21.5,"confirm":null,"ok":true,"list":[1,"a\"b",{}],"t":"Séjour\n"}"#).unwrap();
    assert_eq!(v.get("zone").and_then(Value::as_f64), Some(2.0));
    assert_eq!(v.get("value").and_then(Value::as_f64), Some(21.5));
    assert_eq!(v.get("confirm"), Some(&Value::Null));
    assert_eq!(v.get("t").and_then(Value::as_str), Some("Séjour\n"));
    let again = parse(&v.to_string()).unwrap();
    assert_eq!(again, v);
}

#[test]
fn json_unicode_escape() {
    assert_eq!(parse(r#""\u00e9\ud83d\ude00""#).unwrap(), Value::Str("é😀".into()));
}

#[test]
fn json_hostile_inputs() {
    let deep = "[".repeat(100) + &"]".repeat(100);
    assert_eq!(parse(&deep), Err(JsonError::TooDeep));
    assert_eq!(parse(&"1".repeat(20_000)), Err(JsonError::TooLarge));
    assert!(parse("1e999").is_err());
    assert!(parse(r#"{"a":1,}"#).is_err());
    assert!(parse(r#"{"a" 1}"#).is_err());
    assert!(parse(r#""\ud800""#).is_err());
    assert!(parse("tru").is_err());
    assert!(parse("").is_err());
    assert!(parse("{} x").is_err());
}
