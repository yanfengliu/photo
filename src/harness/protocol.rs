//! The wire protocol: one JSON object per line, each way.
//!
//! The first line of a connection is the session token. After it, each line
//! is a request `{"id": N, "cmd": "...", "params": {...}}`, answered by one
//! line, `{"id": N, "ok": true, "data": {...}}` or
//! `{"id": N, "ok": false, "error": {"code": "...", "message": "..."}}`.

use std::fmt;

use serde_json::{Map, Value, json};

/// The protocol's version, reported by `ping`. Raised when a command or a
/// response changes in a way a client could notice.
pub const PROTOCOL_VERSION: u32 = 1;

/// Why a request failed, as the `code` of its error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    /// The line is not a request: not JSON, not an object, or no `cmd`.
    BadRequest,
    /// The connection's first line was not the session token.
    BadToken,
    /// A parameter is missing, of the wrong type, or out of range.
    InvalidParams,
    /// What the request names (a file, a control) is not there.
    NotFound,
    /// It cannot be done now: a control is disabled, no GPU can render, a
    /// dialog is in the way.
    Unavailable,
    /// The harness does not have this command, or the app lacks what it
    /// needs.
    Unsupported,
    /// Reading or writing a file failed.
    Io,
    /// Waiting gave up.
    Timeout,
    /// The session is shutting down.
    Quitting,
    /// Something went wrong inside the harness or the app.
    Internal,
}

impl Code {
    /// The code as it is written on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BadRequest => "bad_request",
            Self::BadToken => "bad_token",
            Self::InvalidParams => "invalid_params",
            Self::NotFound => "not_found",
            Self::Unavailable => "unavailable",
            Self::Unsupported => "unsupported",
            Self::Io => "io",
            Self::Timeout => "timeout",
            Self::Quitting => "quitting",
            Self::Internal => "internal",
        }
    }
}

/// A failed request: its code and a sentence saying what went wrong.
#[derive(Clone, Debug, PartialEq)]
pub struct CmdError {
    /// The error's code.
    pub code: Code,
    /// What went wrong, for a person or an agent to read.
    pub message: String,
}

impl CmdError {
    /// An error with this code and message.
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    /// A parameter problem.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(Code::InvalidParams, message)
    }

    /// Something named is not there.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(Code::NotFound, message)
    }

    /// It cannot be done now.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(Code::Unavailable, message)
    }

    /// A failure inside the harness.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(Code::Internal, message)
    }

    /// A file could not be read or written.
    pub fn io(message: impl Into<String>) -> Self {
        Self::new(Code::Io, message)
    }
}

impl fmt::Display for CmdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

/// One parsed request.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    /// The client's id, echoed in the response (`null` when it sent none).
    pub id: Value,
    /// The command.
    pub cmd: String,
    /// The parameters (empty when it sent none).
    pub params: Map<String, Value>,
}

/// Parse one request line. A failure carries the id when the line had one,
/// so the error can still be matched to its request.
pub fn parse_request(line: &str) -> Result<Request, (Value, CmdError)> {
    let bad = |id: Value, message: String| (id, CmdError::new(Code::BadRequest, message));
    let value: Value = serde_json::from_str(line)
        .map_err(|error| bad(Value::Null, format!("the line is not JSON: {error}")))?;
    let Value::Object(mut object) = value else {
        return Err(bad(Value::Null, "a request is a JSON object".to_owned()));
    };
    let id = object.remove("id").unwrap_or(Value::Null);
    if !(id.is_null() || id.is_number() || id.is_string()) {
        return Err(bad(Value::Null, "the id must be a number".to_owned()));
    }
    let cmd = match object.remove("cmd") {
        Some(Value::String(cmd)) => cmd,
        Some(_) => return Err(bad(id, "`cmd` must be a string".to_owned())),
        None => return Err(bad(id, "the request has no `cmd`".to_owned())),
    };
    let params = match object.remove("params") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(params)) => params,
        Some(_) => return Err(bad(id, "`params` must be an object".to_owned())),
    };
    if let Some(field) = object.keys().next() {
        return Err(bad(id, format!("a request has no field `{field}`")));
    }
    Ok(Request { id, cmd, params })
}

/// A success response.
pub fn ok_response(id: &Value, data: Value) -> Value {
    json!({ "id": id, "ok": true, "data": data })
}

/// A failure response.
pub fn error_response(id: &Value, error: &CmdError) -> Value {
    json!({
        "id": id,
        "ok": false,
        "error": { "code": error.code.as_str(), "message": error.message },
    })
}

/// A request's parameters, read with errors that name the parameter.
pub struct Params<'a> {
    map: &'a Map<String, Value>,
}

impl<'a> Params<'a> {
    /// The parameters of a request.
    pub fn new(map: &'a Map<String, Value>) -> Self {
        Self { map }
    }

    /// Refuse parameters the command does not take, so a misspelt one is
    /// not silently ignored.
    pub fn only(&self, known: &[&str]) -> Result<(), CmdError> {
        match self.map.keys().find(|key| !known.contains(&key.as_str())) {
            None => Ok(()),
            Some(key) if known.is_empty() => Err(CmdError::invalid(format!(
                "this command takes no parameters, but `{key}` was given"
            ))),
            Some(key) => Err(CmdError::invalid(format!(
                "unknown parameter `{key}`; this command takes {}",
                known
                    .iter()
                    .map(|name| format!("`{name}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    }

    fn get(&self, key: &str) -> Option<&'a Value> {
        self.map.get(key).filter(|value| !value.is_null())
    }

    /// A required string.
    pub fn string(&self, key: &str) -> Result<&'a str, CmdError> {
        self.opt_string(key)?
            .ok_or_else(|| CmdError::invalid(format!("`{key}` is required")))
    }

    /// An optional string.
    pub fn opt_string(&self, key: &str) -> Result<Option<&'a str>, CmdError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(other) => Err(type_error(key, "a string", other)),
        }
    }

    /// An optional boolean.
    pub fn opt_bool(&self, key: &str) -> Result<Option<bool>, CmdError> {
        match self.get(key) {
            None => Ok(None),
            Some(Value::Bool(flag)) => Ok(Some(*flag)),
            Some(other) => Err(type_error(key, "true or false", other)),
        }
    }

    /// An optional whole number of at least zero.
    pub fn opt_u64(&self, key: &str) -> Result<Option<u64>, CmdError> {
        match self.get(key) {
            None => Ok(None),
            Some(value) => value
                .as_u64()
                .map(Some)
                .ok_or_else(|| type_error(key, "a whole number of at least 0", value)),
        }
    }

    /// A required finite number.
    pub fn number(&self, key: &str) -> Result<f64, CmdError> {
        match self.get(key) {
            None => Err(CmdError::invalid(format!("`{key}` is required"))),
            Some(value) => value
                .as_f64()
                .filter(|number| number.is_finite())
                .ok_or_else(|| type_error(key, "a number", value)),
        }
    }

    /// A required point `[x, y]` in logical points.
    pub fn point(&self, key: &str) -> Result<[f32; 2], CmdError> {
        let value = self
            .get(key)
            .ok_or_else(|| CmdError::invalid(format!("`{key}` is required")))?;
        let coordinates: Option<Vec<f64>> = value.as_array().and_then(|items| {
            items
                .iter()
                .map(|item| item.as_f64().filter(|number| number.is_finite()))
                .collect()
        });
        match coordinates.as_deref() {
            Some([x, y]) => Ok([*x as f32, *y as f32]),
            _ => Err(type_error(key, "a point [x, y]", value)),
        }
    }

    /// An optional list of strings, also accepted as one string joined
    /// with `+` (`"cmd+shift"`).
    pub fn opt_names(&self, key: &str) -> Result<Vec<String>, CmdError> {
        match self.get(key) {
            None => Ok(Vec::new()),
            Some(Value::String(joined)) => Ok(joined
                .split('+')
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect()),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| {
                    item.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| type_error(key, "a list of names", item))
                })
                .collect(),
            Some(other) => Err(type_error(key, "a list of names", other)),
        }
    }
}

fn type_error(key: &str, wanted: &str, got: &Value) -> CmdError {
    CmdError::invalid(format!("`{key}` must be {wanted}, not {got}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_has_an_id_a_command_and_parameters() {
        let request =
            parse_request(r#"{"id": 4, "cmd": "click", "params": {"name": "OK"}}"#).unwrap();
        assert_eq!(request.id, json!(4));
        assert_eq!(request.cmd, "click");
        assert_eq!(request.params["name"], json!("OK"));
        let bare = parse_request(r#"{"cmd": "ping"}"#).unwrap();
        assert_eq!((bare.id, bare.params.len()), (Value::Null, 0));
    }

    #[test]
    fn malformed_lines_are_bad_requests_that_keep_their_id() {
        for line in [
            "not json",
            "[1, 2]",
            r#"{"id": 1}"#,
            r#"{"id": 2, "cmd": 7}"#,
            r#"{"id": 3, "cmd": "ping", "params": 5}"#,
            r#"{"id": 4, "cmd": "ping", "extra": 1}"#,
        ] {
            let (_, error) = parse_request(line).unwrap_err();
            assert_eq!(error.code, Code::BadRequest, "{line}");
        }
        let (id, _) = parse_request(r#"{"id": 9}"#).unwrap_err();
        assert_eq!(id, json!(9));
    }

    #[test]
    fn parameters_are_checked_by_name_and_type() {
        let map = json!({"name": 3, "at": [1, 2], "mods": "cmd+shift", "flag": true})
            .as_object()
            .cloned()
            .unwrap();
        let params = Params::new(&map);
        assert_eq!(params.string("name").unwrap_err().code, Code::InvalidParams);
        assert_eq!(
            params.string("missing").unwrap_err().code,
            Code::InvalidParams
        );
        assert_eq!(params.point("at").unwrap(), [1.0, 2.0]);
        assert_eq!(params.opt_names("mods").unwrap(), ["cmd", "shift"]);
        assert_eq!(params.opt_bool("flag").unwrap(), Some(true));
        assert_eq!(params.opt_u64("name").unwrap(), Some(3));
        assert!(params.only(&["name", "at", "mods", "flag"]).is_ok());
        let unknown = params.only(&["name"]).unwrap_err();
        assert!(
            unknown.message.contains("`at`")
                || unknown.message.contains("`mods`")
                || unknown.message.contains("`flag`")
        );
    }

    #[test]
    fn responses_carry_the_id_and_the_code() {
        let error = CmdError::new(Code::Timeout, "still busy");
        assert_eq!(
            error_response(&json!(7), &error),
            json!({"id": 7, "ok": false, "error": {"code": "timeout", "message": "still busy"}})
        );
        assert_eq!(
            ok_response(&json!(1), json!({})),
            json!({"id": 1, "ok": true, "data": {}})
        );
    }
}
