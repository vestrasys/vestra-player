// 实验代码：仅验证 Slint 插件方案，接口和实现与最终成品 API 可能相差很大。
// 视频与音频使用两个 FFmpeg 工具进程，仅粗略同步，不是生产播放后端。
#![forbid(unsafe_code)]

use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 360;
pub const FPS: f64 = 30.0;

#[derive(Clone, Debug)]
pub struct MediaState {
    pub path: Option<PathBuf>,
    pub duration: f64,
    pub position: f64,
    pub paused: bool,
    pub gaussian_enabled: bool,
    pub sigma: f64,
}

pub struct Frame {
    pub rgb: Vec<u8>,
    pub position: f64,
}

#[derive(Default)]
struct Mailbox {
    latest: Option<Frame>,
    ended: bool,
    error: Option<String>,
}

pub struct Backend {
    state: MediaState,
    decoder: Option<Child>,
    audio: Option<Child>,
    reader: Option<JoinHandle<()>>,
    mailbox: Arc<Mutex<Mailbox>>,
    audio_enabled: bool,
    video_filter: String,
    audio_filter: String,
}

impl Backend {
    pub fn new() -> Result<Self, String> {
        Self::with_audio(true)
    }

    pub fn new_silent() -> Result<Self, String> {
        Self::with_audio(false)
    }

    fn with_audio(audio_enabled: bool) -> Result<Self, String> {
        for tool in ["ffmpeg", "ffprobe"] {
            let status = Command::new(tool)
                .arg("-version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|e| format!("找不到 {tool}：{e}"))?;
            if !status.success() {
                return Err(format!("{tool} 无法执行"));
            }
        }
        if audio_enabled {
            let status = Command::new("ffplay")
                .arg("-version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map_err(|e| format!("找不到 ffplay：{e}"))?;
            if !status.success() {
                return Err("ffplay 无法执行".into());
            }
        }
        Ok(Self {
            state: MediaState {
                path: None,
                duration: 0.0,
                position: 0.0,
                paused: true,
                gaussian_enabled: false,
                sigma: 2.0,
            },
            decoder: None,
            audio: None,
            reader: None,
            mailbox: Arc::new(Mutex::new(Mailbox::default())),
            audio_enabled,
            video_filter: String::new(),
            audio_filter: String::new(),
        })
    }

    pub fn snapshot(&self) -> MediaState {
        self.state.clone()
    }

    pub fn load(&mut self, path: PathBuf) -> Result<(), String> {
        let path = path
            .canonicalize()
            .map_err(|e| format!("无法打开文件：{e}"))?;
        if !path.is_file()
            || !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("mp4"))
        {
            return Err("本实验只接受 MP4 文件".into());
        }
        let output = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_type:format=duration",
                "-of",
                "default=noprint_wrappers=1",
            ])
            .arg(&path)
            .output()
            .map_err(|e| format!("探测失败：{e}"))?;
        let result = String::from_utf8_lossy(&output.stdout);
        if !output.status.success() || !result.contains("codec_type=video") {
            return Err("MP4 不包含可播放的视频流".into());
        }
        let duration = result
            .lines()
            .find_map(|line| line.strip_prefix("duration="))
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .ok_or("无法确定 MP4 时长")?;
        self.stop_processes();
        self.state.path = Some(path);
        self.state.duration = duration;
        self.state.position = 0.0;
        self.state.paused = false;
        self.restart()
    }

    pub fn toggle_pause(&mut self) -> Result<(), String> {
        if self.state.path.is_none() {
            return Err("请先打开 MP4".into());
        }
        self.state.paused = !self.state.paused;
        if !self.state.paused && self.state.position >= self.state.duration - 1.0 / FPS {
            self.state.position = 0.0;
        }
        self.restart()
    }

    pub fn seek(&mut self, position: f64) -> Result<(), String> {
        if self.state.path.is_none() {
            return Err("请先打开 MP4".into());
        }
        if !position.is_finite() {
            return Err("跳转位置必须是有限数字".into());
        }
        self.state.position = position.clamp(0.0, (self.state.duration - 1.0 / FPS).max(0.0));
        self.restart()
    }

    pub fn set_gaussian(&mut self, enabled: bool, sigma: f64) -> Result<(), String> {
        if !sigma.is_finite() || !(0.0..=12.0).contains(&sigma) {
            return Err("高斯 sigma 必须在 0 到 12 之间".into());
        }
        self.state.gaussian_enabled = enabled;
        self.state.sigma = sigma;
        if self.state.path.is_some() {
            self.restart()?;
        }
        Ok(())
    }

    pub fn set_processing(&mut self, video: String, audio: String) -> Result<(), String> {
        self.video_filter = video;
        self.audio_filter = audio;
        if self.state.path.is_some() {
            self.restart()?;
        }
        Ok(())
    }

    fn restart(&mut self) -> Result<(), String> {
        self.stop_processes();
        let Some(path) = &self.state.path else {
            return Ok(());
        };
        self.mailbox = Arc::new(Mutex::new(Mailbox::default()));
        let offset = self.state.position;
        let mut filter = format!(
            "scale={WIDTH}:{HEIGHT}:force_original_aspect_ratio=decrease,pad={WIDTH}:{HEIGHT}:(ow-iw)/2:(oh-ih)/2,fps={FPS}"
        );
        if self.state.gaussian_enabled && self.state.sigma > 0.0 {
            filter.push_str(&format!(",gblur=sigma={}:steps=2", self.state.sigma));
        }
        if !self.video_filter.is_empty() {
            filter.push_str(&format!(
                ",setpts=PTS+{offset}/TB,{},setpts=PTS-STARTPTS",
                self.video_filter
            ));
        }
        let mut command = Command::new("ffmpeg");
        command.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
        if !self.state.paused {
            command.arg("-re");
        }
        command
            .args(["-ss", &offset.to_string(), "-i"])
            .arg(path)
            .args(["-map", "0:v:0", "-an", "-sn", "-vf", &filter]);
        if self.state.paused {
            command.args(["-frames:v", "1"]);
        }
        command
            .args(["-pix_fmt", "rgb24", "-f", "rawvideo", "pipe:1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|e| format!("视频进程启动失败：{e}"))?;
        let mut stdout = child.stdout.take().ok_or("缺少视频输出管道")?;
        self.decoder = Some(child);
        let mailbox = Arc::clone(&self.mailbox);
        self.reader = Some(thread::spawn(move || {
            let mut index = 0_u64;
            loop {
                let mut rgb = vec![0; (WIDTH * HEIGHT * 3) as usize];
                match stdout.read_exact(&mut rgb) {
                    Ok(()) => {
                        if let Ok(mut slot) = mailbox.lock() {
                            // 只保留最新一帧；UI 线程不等待管道读取，内存不会随播放增长。
                            slot.latest = Some(Frame {
                                rgb,
                                position: offset + index as f64 / FPS,
                            });
                        } else {
                            break;
                        }
                        index += 1;
                    }
                    Err(error) => {
                        if let Ok(mut slot) = mailbox.lock() {
                            slot.ended = true;
                            if error.kind() != std::io::ErrorKind::UnexpectedEof {
                                slot.error = Some(format!("读取视频帧失败：{error}"));
                            }
                        }
                        break;
                    }
                }
            }
        }));
        if self.audio_enabled && !self.state.paused {
            let mut audio_command = Command::new("ffplay");
            if !self.audio_filter.is_empty() {
                audio_command.args(["-af", &self.audio_filter]);
            }
            match audio_command
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-nostats",
                    "-nodisp",
                    "-autoexit",
                    "-ss",
                    &offset.to_string(),
                    "-i",
                ])
                .arg(path)
                .args(["-vn"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => self.audio = Some(child),
                Err(e) => {
                    self.stop_processes();
                    return Err(format!("音频启动失败：{e}"));
                }
            }
        }
        Ok(())
    }

    pub fn poll(&mut self) -> Result<Option<Frame>, String> {
        let (frame, ended, error) = {
            let mut slot = self.mailbox.lock().map_err(|_| "帧队列锁失败")?;
            (slot.latest.take(), slot.ended, slot.error.take())
        };
        if let Some(error) = error {
            return Err(error);
        }
        if let Some(frame) = &frame {
            self.state.position = frame.position.min(self.state.duration);
        }
        if ended
            && let Some(child) = &mut self.decoder
            && let Some(status) = child.try_wait().map_err(|e| e.to_string())?
        {
            if !status.success() {
                return Err(format!("FFmpeg 视频处理失败：{status}"));
            }
            if !self.state.paused {
                self.state.paused = true;
                self.state.position = self.state.duration;
                if let Some(mut audio) = self.audio.take() {
                    let _ = audio.kill();
                    let _ = audio.wait();
                }
            }
        }
        Ok(frame)
    }

    fn stop_processes(&mut self) {
        for child in [&mut self.decoder, &mut self.audio] {
            if let Some(mut child) = child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }

    pub fn stop(&mut self) {
        self.stop_processes();
        self.mailbox = Arc::new(Mutex::new(Mailbox::default()));
        self.state.paused = true;
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.stop_processes();
    }
}
