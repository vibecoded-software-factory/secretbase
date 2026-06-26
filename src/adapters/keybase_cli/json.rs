//! Helpers for parsing `keybase` JSON output.

use serde_json::Value;

use crate::ports::KeybaseError;

/// Returns the trimmed string value of `key` inside the supplied JSON
/// object, or `None` if the key is missing / not a string.
pub fn opt_str<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// Extracts a [`KeybaseError::Api`] from a Keybase JSON response.
///
/// Both `keybase chat api` and `keybase team api` follow the same
/// convention: success replies have a `result` field, errors have an
/// `error` field with a `message` sub-field plus optional `code`.
/// Some failures simply set the top-level `error` to a string.
pub fn extract_error(v: &Value) -> Option<KeybaseError> {
    let err_obj = v.get("error")?;
    // A present-but-null `error` field (`{"result":…,"error":null}`) is
    // NOT an error — treat it as success.
    if err_obj.is_null() {
        return None;
    }
    let code = err_obj.get("code").and_then(Value::as_i64);
    let message = if let Some(msg) = err_obj.get("message").and_then(Value::as_str) {
        msg.to_string()
    } else if let Some(s) = err_obj.as_str() {
        s.to_string()
    } else {
        // Object without a "message" — fall back to stringifying the
        // whole error blob so the user gets something to debug.
        err_obj.to_string()
    };
    Some(KeybaseError::Api { code, message })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn opt_str_returns_value_for_string_field() {
        let v = json!({"name": "alice"});
        assert_eq!(opt_str(&v, "name"), Some("alice"));
    }

    #[test]
    fn opt_str_returns_none_for_missing_field() {
        let v = json!({"name": "alice"});
        assert_eq!(opt_str(&v, "age"), None);
    }

    #[test]
    fn opt_str_returns_none_for_non_string_field() {
        let v = json!({"age": 42});
        assert_eq!(opt_str(&v, "age"), None);
    }

    #[test]
    fn extract_error_reads_message_field_with_code() {
        let v = json!({"error": {"message": "boom", "code": 7}});
        match extract_error(&v) {
            Some(KeybaseError::Api { code, message }) => {
                assert_eq!(code, Some(7));
                assert_eq!(message, "boom");
            }
            other => panic!("expected Api, got {other:?}"),
        }
    }

    #[test]
    fn extract_error_reads_top_level_string() {
        let v = json!({"error": "bad input"});
        match extract_error(&v) {
            Some(KeybaseError::Api { code, message }) => {
                assert!(code.is_none());
                assert_eq!(message, "bad input");
            }
            other => panic!("expected Api, got {other:?}"),
        }
    }

    #[test]
    fn extract_error_none_when_absent() {
        let v = json!({"result": {}});
        assert!(extract_error(&v).is_none());
    }
}
