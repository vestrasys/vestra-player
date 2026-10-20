//! `plugin.dialogs.confirm`: a system alert (NSAlert) with a confirm and a
//! cancel button and an optional checkbox, as a sheet on the main window.
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

/// Show the alert; `done(confirmed, checked)` runs on the main thread when the
/// user answered. A sheet on `parent` when there is one, else app-modal
/// (`done` then runs before this returns).
#[cfg(target_os = "macos")]
pub fn show(
    options: &Options,
    parent: Option<crate::platform_window::HostWindowId>,
    done: impl Fn(bool, bool) + 'static,
) -> Result<(), RpcError> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSControlStateValueOn};
    use objc2_foundation::NSString;
    let marker = MainThreadMarker::new()
        .ok_or_else(|| RpcError::new("unavailable", "dialogs need the main thread"))?;
    let alert = NSAlert::new(marker);
    alert.setMessageText(&NSString::from_str(&options.message));
    if let Some(detail) = &options.detail {
        alert.setInformativeText(&NSString::from_str(detail));
    }
    let confirm = alert.addButtonWithTitle(&NSString::from_str(&options.confirm));
    confirm.setHasDestructiveAction(options.destructive);
    let cancel = alert.addButtonWithTitle(&NSString::from_str(&options.cancel));
    cancel.setKeyEquivalent(&NSString::from_str("\u{1b}"));
    let checkbox = options.checkbox.as_ref().and_then(|title| {
        alert.setShowsSuppressionButton(true);
        let button = alert.suppressionButton()?;
        button.setTitle(&NSString::from_str(title));
        Some(button)
    });
    let answer = move |response: isize| {
        let checked = checkbox
            .as_ref()
            .is_some_and(|button| button.state() == NSControlStateValueOn);
        done(response == NSAlertFirstButtonReturn, checked);
    };
    match parent.and_then(|id| crate::platform_window::ns_window(id).ok()) {
        Some(window) => {
            let handler = block2::RcBlock::new(answer);
            alert.beginSheetModalForWindow_completionHandler(&window, Some(&handler));
        }
        None => answer(alert.runModal()),
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn show(
    _options: &Options,
    _parent: Option<crate::platform_window::HostWindowId>,
    _done: impl Fn(bool, bool) + 'static,
) -> Result<(), RpcError> {
    Err(RpcError::new(
        "unavailable",
        "system alerts are unavailable on this platform",
    ))
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
