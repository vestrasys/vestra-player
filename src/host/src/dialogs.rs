// 实验原生对话框调度；与最终成品实现可能差异很大，不改变公开 SDK 契约。
use crate::{plugins::Incoming, services::validate_params};
use serde_json::{Value, json};
use std::{collections::HashMap, path::PathBuf, sync::mpsc::Sender};

pub enum Target {
    Plugin { owner: String, request_id: Value },
    AddPlugin,
    Abandoned,
}

pub struct Specification {
    title: String,
    extensions: Vec<String>,
    suggested_name: Option<String>,
}

impl Specification {
    pub fn add_plugin() -> Self {
        Self {
            title: "选择实验插件 manifest.json".into(),
            extensions: vec!["json".into()],
            suggested_name: None,
        }
    }

    pub fn from_request(method: &str, params: &Value) -> Result<Option<Self>, String> {
        if !["dialogs.openMedia", "dialogs.openFile", "dialogs.saveFile"].contains(&method) {
            return Ok(None);
        }
        validate_params(method, params)?;
        if method == "dialogs.openMedia" {
            return Ok(Some(Self {
                title: "打开 MP4".into(),
                extensions: vec!["mp4".into()],
                suggested_name: None,
            }));
        }
        let title = params["title"].as_str().ok_or("missing title")?.to_string();
        let extensions = params["extensions"]
            .as_array()
            .ok_or("missing extensions")?
            .iter()
            .map(|extension| {
                extension
                    .as_str()
                    .map(str::to_string)
                    .ok_or("invalid extension")
            })
            .collect::<Result<Vec<_>, _>>()?;
        let suggested_name = if method == "dialogs.saveFile" {
            Some(
                params["suggestedName"]
                    .as_str()
                    .ok_or("missing suggestedName")?
                    .to_string(),
            )
        } else {
            None
        };
        Ok(Some(Self {
            title,
            extensions,
            suggested_name,
        }))
    }
}

#[derive(Default)]
pub struct Dialogs {
    next_ticket: u64,
    pending: HashMap<u64, Target>,
}

impl Dialogs {
    pub fn begin(
        &mut self,
        specification: Specification,
        target: Target,
        sender: Sender<Incoming>,
    ) -> Result<(), String> {
        if !self.pending.is_empty() {
            return Err("请先完成当前文件对话框".into());
        }
        self.next_ticket += 1;
        let ticket = self.next_ticket;
        // spawn_local posts the first poll to the event loop. The RFD constructor is inside
        // the future, so no AppKit dialog is created while the host timer holds a RefCell borrow.
        slint::spawn_local(async move {
            let dialog = rfd::AsyncFileDialog::new()
                .set_title(specification.title)
                .add_filter("Files", &specification.extensions);
            let result = if let Some(name) = specification.suggested_name {
                dialog.set_file_name(name).save_file().await
            } else {
                dialog.pick_file().await
            };
            let path = result.map(|handle| handle.path().to_path_buf());
            let _ = sender.send(Incoming {
                plugin: String::new(),
                message: json!({"type":"dialog-complete","ticket":ticket,"path":path}),
            });
        })
        .map_err(|error| error.to_string())?;
        self.pending.insert(ticket, target);
        Ok(())
    }

    pub fn complete(&mut self, ticket: u64) -> Option<Target> {
        self.pending.remove(&ticket)
    }

    pub fn abandon(&mut self, owner: &str) {
        for target in self.pending.values_mut() {
            if matches!(target, Target::Plugin {owner: current,..} if current == owner) {
                *target = Target::Abandoned;
            }
        }
    }
}

pub fn selected_path(message: &Value) -> Result<Option<PathBuf>, String> {
    if message["path"].is_null() {
        Ok(None)
    } else {
        message["path"]
            .as_str()
            .map(PathBuf::from)
            .map(Some)
            .ok_or("invalid native dialog result".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requests_keep_native_dialog_validation_and_save_semantics() {
        assert!(
            Specification::from_request("dialogs.openFile", &json!({"title":42,"extensions":[42]}))
                .is_err()
        );
        assert!(
            Specification::from_request(
                "dialogs.saveFile",
                &json!({"title":"Save","extensions":["png"]})
            )
            .is_err()
        );
        let save = Specification::from_request(
            "dialogs.saveFile",
            &json!({"title":"Save","extensions":["png"],"suggestedName":"frame.png"}),
        )
        .unwrap()
        .unwrap();
        assert_eq!(save.suggested_name.as_deref(), Some("frame.png"));
        assert!(
            Specification::from_request("player.snapshot", &json!({}))
                .unwrap()
                .is_none()
        );
    }
}
