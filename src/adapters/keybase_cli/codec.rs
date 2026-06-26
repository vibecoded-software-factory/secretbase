//! JSON request/response codec for the Keybase CLI APIs.
//!
//! `keybase chat api` and `keybase team api` read a single JSON object
//! per invocation from stdin and emit a single JSON object on stdout.
//! These helpers build the request envelopes the rest of the adapter
//! sends.

use serde_json::{Map, Value, json};

use crate::ports::keybase::ReadChannel;

/// Builds a `{"method": NAME}` request with no parameters.
pub fn request_no_params(method: &str) -> Value {
    json!({"method": method})
}

/// Builds a `{"method": NAME, "params": {"options": OPTS}}` request.
pub fn request_with_options(method: &str, options: Value) -> Value {
    json!({
        "method": method,
        "params": { "options": options },
    })
}

/// Encodes a [`ReadChannel`] into the JSON shape `keybase chat api`
/// expects.
///
/// ```json
/// {"name":"alice,bob","members_type":"impteamnative"}
/// // or for a team channel:
/// {"name":"phoenix","members_type":"team","topic_name":"general"}
/// ```
pub fn channel_object(ch: &ReadChannel) -> Value {
    let mut obj = Map::new();
    obj.insert("name".into(), Value::String(ch.name.clone()));
    obj.insert(
        "members_type".into(),
        Value::String(ch.members_type.clone()),
    );
    if let Some(t) = &ch.topic_name {
        obj.insert("topic_name".into(), Value::String(t.clone()));
    }
    Value::Object(obj)
}

/// Serializes a request to a `String` for piping to stdin.
pub fn encode_request(req: &Value) -> String {
    req.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_object_includes_topic_when_present() {
        let ch = ReadChannel {
            name: "phoenix".into(),
            members_type: "team".into(),
            topic_name: Some("general".into()),
        };
        let v = channel_object(&ch);
        assert_eq!(v["name"], "phoenix");
        assert_eq!(v["members_type"], "team");
        assert_eq!(v["topic_name"], "general");
    }

    #[test]
    fn channel_object_omits_topic_for_dm() {
        let ch = ReadChannel {
            name: "alice,bob".into(),
            members_type: "impteamnative".into(),
            topic_name: None,
        };
        let v = channel_object(&ch);
        assert!(v.get("topic_name").is_none());
    }

    #[test]
    fn request_with_options_shape() {
        let req = request_with_options("send", json!({"channel": {"name":"x"}}));
        assert_eq!(req["method"], "send");
        assert_eq!(req["params"]["options"]["channel"]["name"], "x");
    }

    #[test]
    fn encode_request_is_serde_compatible() {
        let s = encode_request(&request_no_params("list"));
        let v: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["method"], "list");
    }
}
