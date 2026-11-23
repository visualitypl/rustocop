//! Reading values from the JSON fixtures in `tests/fixtures`.
use serde::de::DeserializeOwned;
use serde_json::Value;

/// `serde_json::from_value` for a fixture value. A Ruby source is stored as a
/// JSON string when it is valid UTF-8, else as an array of byte values; both
/// read as `Vec<u8>`.
pub fn value<T: DeserializeOwned>(value: &Value) -> T {
    match serde_json::from_value(value.clone()) {
        Ok(read) => read,
        Err(error) => match value {
            Value::String(text) => serde_json::from_value(text.as_bytes().into())
                .unwrap_or_else(|_| panic!("{error}: {value}")),
            _ => panic!("{error}: {value}"),
        },
    }
}
