// 实验受管文件：显式声明依赖，不解析任意 FFmpeg 表达式。
use super::*;
use std::sync::Arc;
pub(super) struct ManagedAsset {
    directory: PathBuf,
    path: PathBuf,
}
impl Drop for ManagedAsset {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
impl Services {
    pub(super) fn stage_files(
        &mut self,
        filter: &str,
        files: Option<&Json>,
    ) -> Result<(String, Vec<Arc<ManagedAsset>>), String> {
        let mut expression = filter.to_string();
        let mut assets = vec![];
        if let Some(files) = files {
            for (name, value) in files.as_object().ok_or("files must be object")? {
                let placeholder = format!("{{{{file:{name}}}}}");
                if !expression.contains(&placeholder) {
                    return Err(format!("declared file {name} has no placeholder"));
                }
                let source = PathBuf::from(value.as_str().ok_or("file path must be string")?)
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                if !source.is_file()
                    || std::fs::metadata(&source).map_err(|e| e.to_string())?.len()
                        > 64 * 1024 * 1024
                {
                    return Err("managed file must be a regular file <=64MiB".into());
                }
                self.next += 1;
                let directory = std::env::temp_dir().join(format!(
                    "vestra-resource-{}-{}",
                    std::process::id(),
                    self.next
                ));
                std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
                let extension = source
                    .extension()
                    .and_then(|e| e.to_str())
                    .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
                    .unwrap_or("data");
                let path = directory.join(format!("resource.{extension}"));
                let asset = Arc::new(ManagedAsset { directory, path });
                std::fs::copy(source, &asset.path).map_err(|e| e.to_string())?;
                let literal = asset
                    .path
                    .to_str()
                    .ok_or("managed file path must be UTF-8")?;
                // Generated temp paths normally contain no punctuation; quote/escape the two FFmpeg parser levels.
                let escaped = literal
                    .replace('\\', "\\\\")
                    .replace(':', "\\:")
                    .replace('\'', "\\'")
                    .replace(',', "\\,")
                    .replace('[', "\\[")
                    .replace(']', "\\]")
                    .replace(';', "\\;")
                    .replace(' ', "\\ ");
                expression = expression.replace(&placeholder, &escaped);
                self.asset_refs.push(Arc::downgrade(&asset));
                assets.push(asset);
            }
        }
        if expression.contains("{{file:") {
            return Err("unresolved file placeholder".into());
        }
        Ok((expression, assets))
    }
}
