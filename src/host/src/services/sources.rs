// 实验公共资源模块；与最终成品 API 可能相差很大。
use super::*;
impl Services {
    pub(super) fn request_sources(
        &mut self,
        owner: &str,
        method: &str,
        p: &Json,
        b: &mut Backend,
    ) -> Result<Option<Json>, String> {
        let value =
            match method {
                "sources.open" => {
                    if self.source.as_ref().is_some_and(|s| s.owner != owner) {
                        return Err("another source owns presentation".into());
                    }
                    let width = p["width"]
                        .as_u64()
                        .filter(|n| *n > 0 && *n <= 1920)
                        .ok_or("invalid width")? as u32;
                    let height = p["height"]
                        .as_u64()
                        .filter(|n| *n > 0 && *n <= 1080)
                        .ok_or("invalid height")? as u32;
                    let sample_rate = p["sampleRate"].as_u64().unwrap_or(44100);
                    if !(8000..=192000).contains(&sample_rate) {
                        return Err("invalid sample rate".into());
                    }
                    let key = text(p, "key")?.to_string();
                    self.close_source();
                    b.stop();
                    self.new_session();
                    self.source = Some(Source {
                        owner: owner.into(),
                        key,
                        width,
                        height,
                        frame: None,
                        frames: 0,
                        paused: false,
                        audio_bytes: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
                        audio_sender: None,
                        audio_reader: None,
                        sample_rate: sample_rate as u32,
                        audio: None,
                    });
                    json!({"sessionId":self.session_id()})
                }
                "sources.setPaused" => {
                    let key = text(p, "key")?;
                    let source = self
                        .source
                        .as_mut()
                        .filter(|s| s.owner == owner && s.key == key)
                        .ok_or("unknown owned source")?;
                    source.paused = p["paused"].as_bool().ok_or("missing paused")?;
                    if source.paused {
                        source.stop_audio();
                    }
                    Json::Null
                }
                "sources.present" => {
                    if text(p, "sessionId")? != self.session_id() {
                        return Err("stale session".into());
                    }
                    let s = self
                        .source
                        .as_mut()
                        .filter(|s| s.owner == owner && s.key == p["key"].as_str().unwrap_or(""))
                        .ok_or("unknown owned source")?;
                    use base64::Engine;
                    let rgb = base64::engine::general_purpose::STANDARD
                        .decode(text(p, "rgb")?)
                        .map_err(|e| e.to_string())?;
                    if rgb.len() != (s.width * s.height * 3) as usize {
                        return Err("RGB frame size mismatch".into());
                    }
                    if s.paused {
                        return Err("source is paused".into());
                    }
                    if let Some(pcm) = p["pcm"].as_str() {
                        let bytes = base64::engine::general_purpose::STANDARD
                            .decode(pcm)
                            .map_err(|e| e.to_string())?;
                        if bytes.len() % 4 != 0 || bytes.len() > s.sample_rate as usize * 4 / 5 {
                            return Err("invalid stereo PCM16LE".into());
                        }
                        if s.audio.is_none() {
                            let mut child = Command::new("ffplay")
                                .args([
                                    "-hide_banner",
                                    "-loglevel",
                                    "error",
                                    "-nodisp",
                                    "-autoexit",
                                    "-f",
                                    "s16le",
                                    "-ar",
                                    &s.sample_rate.to_string(),
                                    "-ch_layout",
                                    "stereo",
                                    "-probesize",
                                    "32",
                                    "-analyzeduration",
                                    "0",
                                    "-i",
                                    "pipe:0",
                                ])
                                .stdin(Stdio::piped())
                                .stdout(Stdio::null())
                                .stderr(Stdio::inherit())
                                .spawn()
                                .map_err(|e| e.to_string())?;
                            let mut pipe = child.stdin.take().ok_or("audio stdin unavailable")?;
                            let (sender, receiver) = std::sync::mpsc::sync_channel::<Vec<u8>>(64);
                            let written = s.audio_bytes.clone();
                            s.audio_reader = Some(std::thread::spawn(move || {
                                use std::io::Write;
                                for bytes in receiver {
                                    if pipe.write_all(&bytes).is_err() {
                                        break;
                                    }
                                    written.fetch_add(
                                        bytes.len() as u64,
                                        std::sync::atomic::Ordering::Relaxed,
                                    );
                                }
                            }));
                            s.audio = Some(child);
                            s.audio_sender = Some(sender);
                        }
                        s.audio_sender
                            .as_ref()
                            .ok_or("audio unavailable")?
                            .try_send(bytes)
                            .map_err(|e| format!("audio backpressure: {e}"))?;
                    }
                    s.frame = Some(rgb);
                    s.frames += 1;
                    Json::Null
                }
                "sources.close" => {
                    if self.source.as_ref().is_some_and(|s| {
                        s.owner == owner && s.key == p["key"].as_str().unwrap_or("")
                    }) {
                        self.close_source();
                        self.new_session();
                    }
                    Json::Null
                }

                _ => return Ok(None),
            };
        Ok(Some(value))
    }
}
