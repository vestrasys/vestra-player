// 实验公共资源模块；与最终成品 API 可能相差很大。
use super::*;
impl Services {
    pub(super) fn request_effects(
        &mut self,
        owner: &str,
        method: &str,
        p: &Json,
        b: &mut Backend,
    ) -> Result<Option<Json>, String> {
        let value = match method {
            "capabilities" => {
                let mut caps = vec![
                    "api.probe-0.3",
                    "effects.managed-files",
                    "effects.video",
                    "effects.audio",
                    "media.tasks",
                    "sources.rgb-pcm",
                    "plugins.management",
                    "commands.menus",
                    "sessions",
                ];
                if self.subtitle_supported {
                    caps.push("effects.subtitle");
                }
                json!(caps)
            }
            "player.filters.setGaussian" => {
                if self
                    .legacy_gaussian_owner
                    .as_deref()
                    .is_some_and(|current| current != owner)
                {
                    return Err("legacy Gaussian effect is owned by another plugin".into());
                }
                let enabled = p["enabled"].as_bool().ok_or("missing enabled")?;
                let sigma = p["sigma"].as_f64().ok_or("missing sigma")?;
                b.set_gaussian(enabled, sigma)?;
                self.legacy_gaussian_owner = if enabled { Some(owner.into()) } else { None };
                Json::Null
            }
            "effects.set" => {
                let key = text(p, "key")?.to_string();
                let kind = text(p, "kind")?;
                if !["video", "audio", "subtitle"].contains(&kind) {
                    return Err("invalid effect kind".into());
                }
                if kind == "subtitle" && !self.subtitle_supported {
                    return Err("subtitle capability unavailable: FFmpeg requires libass".into());
                }
                let filter = text(p, "filter")?;
                if filter.len() > 16384 || filter.contains('\0') {
                    return Err("invalid filter".into());
                }
                let session = p["sessionId"].as_str().map(str::to_string);
                if kind == "subtitle" && session.is_none() {
                    return Err("subtitle requires sessionId".into());
                }
                if session.as_ref().is_some_and(|s| s != &self.session_id()) {
                    return Err("stale session".into());
                }
                if kind == "subtitle" && p["files"].as_object().is_none_or(|files| files.is_empty())
                {
                    return Err("probe-0.3 subtitle effects require managed files and {{file:name}} placeholders".into());
                }
                let (filter, assets) = self.stage_files(filter, p.get("files"))?;
                validate_filter(kind, &filter)?;
                let old = self.effects.clone();
                let effect = Effect {
                    owner: owner.into(),
                    key: key.clone(),
                    kind: kind.into(),
                    filter,
                    assets,
                    session,
                };
                if let Some(existing) = self
                    .effects
                    .iter_mut()
                    .find(|e| e.owner == owner && e.key == key)
                {
                    *existing = effect;
                } else {
                    self.effects.push(effect);
                }
                if let Err(e) = self.apply(b) {
                    self.effects = old;
                    return Err(e);
                }
                Json::Null
            }
            "effects.remove" => {
                let key = text(p, "key")?;
                self.effects.retain(|e| !(e.owner == owner && e.key == key));
                self.apply(b)?;
                Json::Null
            }
            "effects.list" => json!(
                self.effects
                    .iter()
                    .map(|e| json!({"owner":e.owner,"key":e.key,"kind":e.kind,"filter":e.filter}))
                    .collect::<Vec<_>>()
            ),
            "effects.snapshot" => {
                self.next += 1;
                let id = format!("snapshot-{}", self.next);
                let (video, audio) = self.chains(p["includeSubtitles"] != false, b);
                let assets = self
                    .effects
                    .iter()
                    .filter(|effect| p["includeSubtitles"] != false || effect.kind != "subtitle")
                    .flat_map(|effect| effect.assets.iter().cloned())
                    .collect();
                self.snapshots.insert(
                    id.clone(),
                    ProcessingSnapshot {
                        owner: owner.into(),
                        video,
                        audio,
                        assets,
                    },
                );
                json!({"id":id})
            }
            "effects.releaseSnapshot" => {
                let id = text(p, "id")?;
                if self.snapshots.get(id).is_some_and(|s| s.owner == owner) {
                    self.snapshots.remove(id);
                }
                Json::Null
            }

            _ => return Ok(None),
        };
        Ok(Some(value))
    }
}
