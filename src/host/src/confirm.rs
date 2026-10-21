//! `plugin.dialogs.confirm`: a system alert with a confirm and a cancel button
//! and an optional checkbox. The options are checked here; showing the alert
//! is `platform::confirm` (an NSAlert sheet on macOS, a message box on
//! Windows).
#![forbid(unsafe_code)]

use crate::RpcError;
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub message: String,
    pub detail: Option<String>,
    pub confirm: String,
    pub cancel: String,
    pub destructive: bool,
    pub checkbox: Option<String>,
}

/// `message` 1–200, `detail` ≤ 2000, button and checkbox labels 1–40 characters.
pub fn parse(params: &Value) -> Result<Options, RpcError> {
    let text = |key: &str, min: usize, max: usize| -> Result<Option<String>, RpcError> {
        match &params[key] {
            Value::Null => Ok(None),
            Value::String(text)
                if (min..=max).contains(&text.chars().count()) && !text.contains('\0') =>
            {
                Ok(Some(text.clone()))
            }
            _ => Err(RpcError::invalid(format!(
                "{key} requires {min}..={max} characters"
            ))),
        }
    };
    let required = |key: &str, max: usize| {
        text(key, 1, max)?.ok_or_else(|| RpcError::invalid(format!("{key} is required")))
    };
    let destructive = match &params["destructive"] {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        _ => return Err(RpcError::invalid("destructive requires a boolean")),
    };
    Ok(Options {
        message: required("message", 200)?,
        detail: text("detail", 0, 2000)?.filter(|detail| !detail.is_empty()),
        confirm: required("confirm", 40)?,
        cancel: text("cancel", 1, 40)?.unwrap_or_else(|| "取消".into()),
        destructive,
        checkbox: text("checkbox", 1, 40)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn options_are_bounded_and_cancel_defaults() {
        let options = parse(&json!({"message":"清空播放列表？","confirm":"清空"})).unwrap();
        assert_eq!(options.cancel, "取消");
        assert_eq!((options.detail, options.checkbox), (None, None));
        assert!(!options.destructive);
        let full = parse(
            &json!({"message":"卸载？","detail":"数据也删除","confirm":"卸载",
            "cancel":"保留","destructive":true,"checkbox":"同时删除数据"}),
        )
        .unwrap();
        assert!(full.destructive && full.checkbox.as_deref() == Some("同时删除数据"));
        for bad in [
            json!({"confirm":"好"}),
            json!({"message":"","confirm":"好"}),
            json!({"message":"x".repeat(201),"confirm":"好"}),
            json!({"message":"好？","confirm":"x".repeat(41)}),
            json!({"message":"好？","confirm":"好","cancel":""}),
            json!({"message":"好？","confirm":"好","detail":"x".repeat(2001)}),
            json!({"message":"好？","confirm":"好","checkbox":""}),
            json!({"message":"好？","confirm":"好","destructive":"yes"}),
        ] {
            assert_eq!(parse(&bad).unwrap_err().code, "invalid", "{bad}");
        }
        // 200 CJK characters are 200 characters, not 600 bytes.
        assert!(parse(&json!({"message":"中".repeat(200),"confirm":"好"})).is_ok());
    }
}
