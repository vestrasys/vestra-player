//! Requests that reach the operating system on the plugin's behalf:
//! `system.reveal` / `openPath` / `openUrl` (show files, open them with the
//! default application, open links in the browser), `secrets.get` / `set` /
//! `delete` (the system's secret store, capability `secrets`) and
//! `app.system` (system facts and displays for `plugin.app.system()`).
//!
//! Arguments are checked here; the system calls are `platform::*`. They can
//! block (Finder, keychain prompts), so they run on a worker thread and the
//! reply comes back as an `Event::Reply`.
#![forbid(unsafe_code)]
use super::*;
use std::sync::OnceLock;

pub const MAX_REVEAL: usize = 256;
pub const MAX_URL: usize = 8192;
pub const MAX_SECRET_BYTES: usize = 1024;

/// An absolute path that exists (`missing` otherwise).
pub fn existing(value: &Value) -> std::result::Result<PathBuf, RpcError> {
    let text = value
        .as_str()
        .filter(|text| !text.is_empty() && text.len() <= 4096 && !text.contains('\0'))
        .ok_or_else(|| RpcError::invalid("paths are non-empty strings"))?;
    let path = PathBuf::from(text);
    if !path.is_absolute() {
        return Err(RpcError::invalid("paths must be absolute"));
    }
    if std::fs::symlink_metadata(&path).is_err() {
        return Err(RpcError::new("missing", format!("{text} does not exist")));
    }
    Ok(path)
}

/// 1–256 paths; those that do not exist are skipped, `missing` when none does.
pub fn reveal_paths(params: &Value) -> std::result::Result<Vec<PathBuf>, RpcError> {
    let list = params["paths"]
        .as_array()
        .filter(|list| !list.is_empty() && list.len() <= MAX_REVEAL)
        .ok_or_else(|| RpcError::invalid("reveal takes 1–256 paths"))?;
    let mut present = Vec::new();
    let mut missing = Vec::new();
    for value in list {
        match existing(value) {
            Ok(path) if !present.contains(&path) => present.push(path),
            Ok(_) => {}
            Err(error) if error.code == "missing" => missing.push(error.message),
            Err(error) => return Err(error),
        }
    }
    if present.is_empty() {
        return Err(RpcError::new("missing", missing.join("; ")));
    }
    Ok(present)
}

/// A well-formed `http:` or `https:` address, normalized (it then starts
/// with its scheme, never with `-`).
pub fn http_url(params: &Value) -> std::result::Result<String, RpcError> {
    let text = params["url"]
        .as_str()
        .filter(|text| text.len() <= MAX_URL)
        .ok_or_else(|| RpcError::invalid("url must be a well-formed http(s) address"))?;
    let parsed = url::Url::parse(text)
        .map_err(|_| RpcError::invalid("url must be a well-formed http(s) address"))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(RpcError::invalid("only http and https addresses open"));
    }
    Ok(parsed.into())
}

/// Secret names: 1–64 of `a-z 0-9 . _ -`.
pub fn secret_name(params: &Value) -> std::result::Result<&str, RpcError> {
    params["name"]
        .as_str()
        .filter(|name| {
            (1..=64).contains(&name.len())
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        })
        .ok_or_else(|| RpcError::invalid("secret names are 1–64 of a-z, 0-9, ., _ and -"))
}

/// Secret values: 1–1024 bytes of UTF-8 without NUL.
pub fn secret_value(params: &Value) -> std::result::Result<&str, RpcError> {
    params["value"]
        .as_str()
        .filter(|value| (1..=MAX_SECRET_BYTES).contains(&value.len()) && !value.contains('\0'))
        .ok_or_else(|| {
            RpcError::invalid("secret values are 1–1024 bytes of text; delete() removes one")
        })
}

/// `plugin.app.system()` without the displays.
pub fn facts_json(facts: &platform::OsFacts) -> Value {
    json!({"os":facts.os,"name":facts.name,"version":facts.version,"build":facts.build,
        "model":facts.model,"arch":std::env::consts::ARCH})
}

/// `displays` of `plugin.app.system()`.
pub fn displays_json(displays: &[platform::Display]) -> Value {
    displays
        .iter()
        .map(|d| {
            json!({"name":d.name,"width":d.width,"height":d.height,"scale":d.scale,
                "refreshRate":d.refresh_rate,"primary":d.primary})
        })
        .collect()
}

/// Computed once per host process, off the UI thread.
static FACTS: OnceLock<platform::OsFacts> = OnceLock::new();

impl Host {
    pub(super) fn system_dispatch(
        &mut self,
        reply: &Reply,
        method: &str,
        params: &Value,
    ) -> std::result::Result<Option<Value>, RpcError> {
        type Job = Box<dyn FnOnce() -> std::result::Result<Value, RpcError> + Send>;
        let job: Job = match method {
            "system.reveal" => {
                let paths = reveal_paths(params)?;
                Box::new(move || platform::reveal(&paths).map(|()| Value::Null))
            }
            "system.openPath" => {
                let path = existing(&params["path"])?;
                Box::new(move || platform::open_path(&path).map(|()| Value::Null))
            }
            "system.openUrl" => {
                let url = http_url(params)?;
                Box::new(move || platform::open_url(&url).map(|()| Value::Null))
            }
            "secrets.get" | "secrets.set" | "secrets.delete" => {
                let service = self
                    .plugins
                    .get(&reply.owner)
                    .map(|plugin| plugin.data_key.secret_service())
                    .ok_or_else(|| RpcError::new("disabled", "plugin no longer running"))?;
                let name = secret_name(params)?.to_owned();
                // The host records the names, so uninstalling can delete them.
                let index = self.packages.secrets.clone();
                match method {
                    "secrets.get" => Box::new(move || {
                        platform::secret_get(&service, &name)
                            .map(|value| value.map_or(Value::Null, Value::String))
                    }),
                    "secrets.set" => {
                        let value = secret_value(params)?.to_owned();
                        index.reserve(&service, &name)?;
                        Box::new(move || {
                            platform::secret_set(&service, &name, &value).map(|()| Value::Null)
                        })
                    }
                    _ => {
                        let generation = index.generation(&service, &name);
                        Box::new(move || {
                            platform::secret_delete(&service, &name)?;
                            index.release(&service, &name, generation);
                            Ok(Value::Null)
                        })
                    }
                }
            }
            "app.system" => {
                // Displays are read on the UI thread, where the windows live.
                let displays = if self.headless {
                    Value::Array(Vec::new())
                } else {
                    displays_json(&platform::displays())
                };
                Box::new(move || {
                    let mut value = facts_json(FACTS.get_or_init(platform::os_facts));
                    value["displays"] = displays;
                    Ok(value)
                })
            }
            _ => return Err(RpcError::new("unsupported", "unknown system method")),
        };
        let events = self.events.clone();
        let reply = reply.clone();
        std::thread::Builder::new()
            .name("system-request".into())
            .spawn(move || {
                let _ = events.send(Event::Reply(reply, job()));
            })
            .map_err(|error| RpcError::new("busy", error.to_string()))?;
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_must_be_absolute_and_present_and_reveal_skips_the_missing() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.mkv");
        std::fs::write(&file, b"x").unwrap();
        let text = file.to_str().unwrap();
        assert_eq!(existing(&json!(text)).unwrap(), file);
        assert_eq!(
            existing(&json!("relative.mkv")).unwrap_err().code,
            "invalid"
        );
        assert_eq!(existing(&json!("")).unwrap_err().code, "invalid");
        let gone = dir.path().join("gone.mkv");
        assert_eq!(
            existing(&json!(gone.to_str().unwrap())).unwrap_err().code,
            "missing"
        );
        let paths = reveal_paths(&json!({"paths":[text, gone.to_str().unwrap(), text]})).unwrap();
        assert_eq!(paths, std::slice::from_ref(&file));
        assert_eq!(
            reveal_paths(&json!({"paths":[gone.to_str().unwrap()]}))
                .unwrap_err()
                .code,
            "missing"
        );
        assert_eq!(
            reveal_paths(&json!({"paths":[]})).unwrap_err().code,
            "invalid"
        );
        let many = vec![text; MAX_REVEAL + 1];
        assert_eq!(
            reveal_paths(&json!({ "paths": many })).unwrap_err().code,
            "invalid"
        );
    }

    #[test]
    fn only_well_formed_http_addresses_open_and_they_are_normalized() {
        assert_eq!(
            http_url(&json!({"url":"https://example.com/a b"})).unwrap(),
            "https://example.com/a%20b"
        );
        assert_eq!(
            http_url(&json!({"url":"HTTP://Example.com"})).unwrap(),
            "http://example.com/"
        );
        for bad in [
            "file:///etc/passwd",
            "-a Calculator",
            "javascript:alert(1)",
            "",
        ] {
            assert_eq!(
                http_url(&json!({ "url": bad })).unwrap_err().code,
                "invalid",
                "{bad}"
            );
        }
        let long = format!("https://example.com/{}", "x".repeat(MAX_URL));
        assert_eq!(
            http_url(&json!({ "url": long })).unwrap_err().code,
            "invalid"
        );
    }

    #[test]
    fn secret_names_and_values_are_bounded() {
        assert_eq!(
            secret_name(&json!({"name":"opensubtitles-api-key"})).unwrap(),
            "opensubtitles-api-key"
        );
        for bad in [
            json!(""),
            json!("API"),
            json!("a/b"),
            json!("x".repeat(65)),
            json!(3),
        ] {
            assert_eq!(
                secret_name(&json!({ "name": bad })).unwrap_err().code,
                "invalid"
            );
        }
        assert_eq!(secret_value(&json!({"value":"密钥"})).unwrap(), "密钥");
        for bad in [
            json!(""),
            json!("a\0b"),
            json!("x".repeat(1025)),
            json!(null),
        ] {
            assert_eq!(
                secret_value(&json!({ "value": bad })).unwrap_err().code,
                "invalid"
            );
        }
    }

    #[test]
    fn displays_are_plain_json() {
        let value = displays_json(&[platform::Display {
            name: Some("Built-in Retina Display".into()),
            width: 3024,
            height: 1964,
            scale: 2.,
            refresh_rate: Some(120.),
            primary: true,
        }]);
        assert_eq!(
            value,
            json!([{"name":"Built-in Retina Display","width":3024,"height":1964,
                "scale":2.0,"refreshRate":120.0,"primary":true}])
        );
    }

    #[test]
    fn facts_carry_the_system_and_the_architecture() {
        let value = facts_json(&platform::os_facts());
        assert!(["macos", "windows", "linux", "other"].contains(&value["os"].as_str().unwrap()));
        assert_eq!(value["arch"], std::env::consts::ARCH);
        assert!(value["name"].is_string());
    }
}
