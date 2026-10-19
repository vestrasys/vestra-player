// 实验公共资源模块；与最终成品 API 可能相差很大。
use super::*;
impl Services {
    pub(super) fn request_jobs(
        &mut self,
        owner: &str,
        method: &str,
        p: &Json,
        b: &mut Backend,
    ) -> Result<Option<Json>, String> {
        let value = match method {
            "media.start" => {
                let input = PathBuf::from(text(p, "input")?)
                    .canonicalize()
                    .map_err(|e| e.to_string())?;
                let output = PathBuf::from(text(p, "output")?);
                if !output.is_absolute() || output.exists() {
                    return Err("output must be new absolute path".into());
                }
                let format = text(p, "format")?;
                if !["clip", "image", "audio"].contains(&format) {
                    return Err("invalid format".into());
                }
                if p["processingSnapshot"].is_string() && p["processing"] == "current" {
                    return Err("choose processing or processingSnapshot".into());
                }
                let (video, audio, snapshot, assets) =
                    if let Some(id) = p["processingSnapshot"].as_str() {
                        let s = self
                            .snapshots
                            .get(id)
                            .filter(|s| s.owner == owner)
                            .ok_or("unknown owned processing snapshot")?;
                        (s.video.clone(), s.audio.clone(), true, s.assets.clone())
                    } else {
                        let (v, a) = self.chains(p["includeSubtitles"] != false, b);
                        (
                            v,
                            a,
                            p["processing"] == "current",
                            if p["processing"] == "current" {
                                self.effects
                                    .iter()
                                    .filter(|effect| {
                                        p["includeSubtitles"] != false || effect.kind != "subtitle"
                                    })
                                    .flat_map(|effect| effect.assets.iter().cloned())
                                    .collect()
                            } else {
                                vec![]
                            },
                        )
                    };
                let start = finite(p, "start", 0.)?;
                if start < 0. {
                    return Err("start must be nonnegative".into());
                }
                let mut command = Command::new("ffmpeg");
                command
                    .args([
                        "-hide_banner",
                        "-loglevel",
                        "error",
                        "-nostdin",
                        "-n",
                        "-ss",
                        &start.to_string(),
                        "-i",
                    ])
                    .arg(input);
                if let Some(d) = p["duration"].as_f64() {
                    if !d.is_finite() || d <= 0. {
                        return Err("invalid duration".into());
                    }
                    command.args(["-t", &d.to_string()]);
                }
                let chain = |live: &str, explicit: Option<&str>| {
                    [if snapshot { live } else { "" }, explicit.unwrap_or("")]
                        .into_iter()
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(",")
                };
                let vf = chain(&video, p["videoFilter"].as_str());
                let af = chain(&audio, p["audioFilter"].as_str());
                if !vf.is_empty() {
                    command.args([
                        "-vf",
                        &format!("setpts=PTS+{start}/TB,{vf},setpts=PTS-STARTPTS"),
                    ]);
                }
                if !af.is_empty() {
                    command.args(["-af", &af]);
                }
                match format {
                    "image" => {
                        command.args(["-frames:v", "1", "-an"]);
                    }
                    "audio" => {
                        command.args(["-vn", "-c:a", "pcm_s16le"]);
                    }
                    _ => {
                        command.args(["-c:v", "libx264", "-preset", "ultrafast", "-c:a", "aac"]);
                    }
                }
                self.next += 1;
                let id = format!("job-{}", self.next);
                let directory = output.parent().ok_or("missing output parent")?;
                let filename = output
                    .file_name()
                    .ok_or("missing output filename")?
                    .to_string_lossy();
                let working =
                    directory.join(format!(".vestra-{}-{id}-{filename}", std::process::id()));
                let diagnostic = working.with_extension("stderr");
                let progress = working.with_extension("progress");
                let errors = std::fs::File::create(&diagnostic).map_err(|e| e.to_string())?;
                command.arg("-progress").arg(&progress);
                let child = command
                    .arg(&working)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::from(errors))
                    .spawn()
                    .map_err(|e| e.to_string())?;
                self.jobs.insert(
                    id.clone(),
                    Job {
                        owner: owner.into(),
                        child: Some(child),
                        state: "running".into(),
                        error: None,
                        output,
                        working,
                        diagnostic,
                        progress,
                        progress_seconds: 0.,
                        assets,
                    },
                );
                json!({"id":id,"state":"running"})
            }
            "media.status" | "media.cancel" => {
                let id = text(p, "id")?;
                let job = self
                    .jobs
                    .get_mut(id)
                    .filter(|j| j.owner == owner)
                    .ok_or("unknown owned job")?;
                if method == "media.cancel" {
                    cancel(job);
                    Json::Null
                } else {
                    let progress = std::fs::read_to_string(&job.progress).unwrap_or_default();
                    job.progress_seconds = progress
                        .lines()
                        .filter_map(|line| line.strip_prefix("out_time_us="))
                        .filter_map(|n| n.parse::<f64>().ok())
                        .next_back()
                        .unwrap_or(job.progress_seconds * 1_000_000.)
                        / 1_000_000.;
                    if let Some(child) = job.child.as_mut()
                        && let Some(status) = child.try_wait().map_err(|e| e.to_string())?
                    {
                        let publish = if status.success() {
                            std::fs::hard_link(&job.working, &job.output).map_err(|e| e.to_string())
                        } else {
                            Err(format!(
                                "FFmpeg exited {status}: {}",
                                std::fs::read_to_string(&job.diagnostic).unwrap_or_default()
                            ))
                        };
                        match publish {
                            Ok(()) => job.state = "succeeded".into(),
                            Err(error) => {
                                job.state = "failed".into();
                                job.error = Some(error);
                            }
                        }
                        let _ = std::fs::remove_file(&job.working);
                        let _ = std::fs::remove_file(&job.diagnostic);
                        job.child = None;
                        job.assets.clear();
                        let _ = std::fs::remove_file(&job.progress);
                    }
                    let mut state =
                        json!({"id":id,"state":job.state,"progressSeconds":job.progress_seconds});
                    if let Some(error) = &job.error {
                        state["error"] = json!(error);
                    }
                    state
                }
            }

            _ => return Ok(None),
        };
        Ok(Some(value))
    }
}
