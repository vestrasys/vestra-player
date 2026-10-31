//! Render options `plugin.video.render()` (R-V10): HDR tone mapping, user
//! GLSL shaders and motion interpolation, as mpv GPU renderer options. One
//! owner at a time (like the display layer); the owner's release or disable
//! writes mpv's defaults back. Only the GPU renderer applies them; the
//! software renderer refuses with `unsupported` (the caller checks). They act
//! at render time, so snapshots and exports never contain them.
//!
//! mpv's `glsl-shaders` is written only here: the owner's `shaders`, then
//! the shader channel's joined list (`video_shaders`, any number of
//! plugins, player 0.5.20).
#![forbid(unsafe_code)]

use crate::host::RpcError;
use serde_json::{Value, json};
use std::path::Path;

pub const MAX_SHADERS: usize = 16;
pub const TONE_MAPPINGS: [&str; 7] = [
    "auto", "off", "bt.2390", "hable", "mobius", "reinhard", "clip",
];

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub tone_mapping: &'static str,
    /// Nits, 100–400.
    pub target_peak: f64,
    pub peak_detect: bool,
    pub shaders: Vec<String>,
    pub interpolation: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            tone_mapping: "auto",
            target_peak: 203.,
            peak_detect: true,
            shaders: Vec::new(),
            interpolation: false,
        }
    }
}
impl Settings {
    /// `values` merged onto `self`; unknown fields, values out of range,
    /// relative or missing shader files are `invalid` and change nothing.
    pub fn merged(&self, values: &Value) -> Result<Self, RpcError> {
        let values = values
            .as_object()
            .ok_or_else(|| RpcError::invalid("values must be an object"))?;
        let boolean = |key: &str, value: &Value| {
            value
                .as_bool()
                .ok_or_else(|| RpcError::invalid(format!("{key} must be a boolean")))
        };
        let mut next = self.clone();
        for (key, value) in values {
            match key.as_str() {
                "toneMapping" => {
                    next.tone_mapping = TONE_MAPPINGS
                        .into_iter()
                        .find(|name| value.as_str() == Some(name))
                        .ok_or_else(|| {
                            RpcError::invalid(format!(
                                "toneMapping must be one of {}",
                                TONE_MAPPINGS.join(", ")
                            ))
                        })?
                }
                "targetPeak" => {
                    next.target_peak = value
                        .as_f64()
                        .filter(|v| v.is_finite() && (100. ..=400.).contains(v))
                        .ok_or_else(|| {
                            RpcError::invalid("targetPeak must be a number in 100..400")
                        })?
                }
                "peakDetect" => next.peak_detect = boolean(key, value)?,
                "interpolation" => next.interpolation = boolean(key, value)?,
                "shaders" => next.shaders = shaders(value)?,
                other => return Err(RpcError::invalid(format!("unknown render field {other}"))),
            }
        }
        Ok(next)
    }
}

fn shaders(value: &Value) -> Result<Vec<String>, RpcError> {
    let list = value
        .as_array()
        .ok_or_else(|| RpcError::invalid("shaders must be an array of paths"))?;
    if list.len() > MAX_SHADERS {
        return Err(RpcError::invalid(format!("at most {MAX_SHADERS} shaders")));
    }
    list.iter()
        .map(|path| {
            let path = path
                .as_str()
                .ok_or_else(|| RpcError::invalid("shaders must be an array of paths"))?;
            if !Path::new(path).is_absolute() {
                return Err(RpcError::invalid(format!(
                    "shader path must be absolute: {path}"
                )));
            }
            if !Path::new(path).is_file() {
                return Err(RpcError::invalid(format!(
                    "shader file does not exist: {path}"
                )));
            }
            Ok(path.to_owned())
        })
        .collect()
}

/// What the options write on the player (`Player` in tests).
pub trait Player {
    fn set(&mut self, name: &str, value: &str) -> Result<(), String>;
    /// Replace mpv's `glsl-shaders` list.
    fn shaders(&mut self, paths: &[String]) -> Result<(), String>;
}

/// The mpv option values for `settings`; `None`: mpv's own defaults.
/// `refresh` is the display rate interpolation resamples to.
pub fn properties(settings: Option<&Settings>, refresh: f64) -> Vec<(&'static str, String)> {
    let Some(s) = settings else {
        return [
            ("tone-mapping", "auto"),
            ("gamut-mapping-mode", "auto"),
            ("hdr-compute-peak", "auto"),
            ("target-peak", "auto"),
            ("interpolation", "no"),
            ("video-sync", "audio"),
            ("display-fps-override", "0"),
        ]
        .map(|(name, value)| (name, value.to_owned()))
        .to_vec();
    };
    let yes = |on: bool| if on { "yes" } else { "no" }.to_owned();
    // "off": no curve and no gamut compression; out-of-range light is clipped.
    let off = s.tone_mapping == "off";
    vec![
        (
            "tone-mapping",
            if off { "clip" } else { s.tone_mapping }.into(),
        ),
        (
            "gamut-mapping-mode",
            if off { "clip" } else { "auto" }.into(),
        ),
        ("hdr-compute-peak", yes(s.peak_detect && !off)),
        ("target-peak", s.target_peak.to_string()),
        ("interpolation", yes(s.interpolation)),
        (
            "video-sync",
            if s.interpolation {
                "display-resample"
            } else {
                "audio"
            }
            .into(),
        ),
        (
            "display-fps-override",
            if s.interpolation {
                refresh.to_string()
            } else {
                "0".into()
            },
        ),
    ]
}

/// Who owns the render options and what it last asked for.
#[derive(Debug, Default)]
pub struct Options {
    owner: Option<u64>,
    settings: Settings,
    /// A shader failure was reported for the current settings.
    reported: bool,
    /// The shader channel's joined list (`video_shaders`), written to mpv
    /// after `settings.shaders`: this is the only writer of `glsl-shaders`.
    channel: Vec<String>,
}
impl Options {
    pub fn owner(&self) -> Option<u64> {
        self.owner
    }
    pub fn settings(&self) -> &Settings {
        &self.settings
    }
    /// `video.render.set { values }` → `{ state }`: takes the options when
    /// free (`conflict` while another plugin holds them), merges, writes mpv.
    /// mpv keeps them across loads; without ready media they are `waiting`.
    pub fn set(
        &mut self,
        owner: u64,
        values: &Value,
        ready: bool,
        refresh: f64,
        player: &mut impl Player,
    ) -> Result<Value, RpcError> {
        if self.owner.is_some_and(|held| held != owner) {
            return Err(RpcError::new(
                "conflict",
                "another plugin owns the render options",
            ));
        }
        let next = self.settings.merged(values)?;
        self.owner = Some(owner);
        self.write(Some(next), refresh, player)?;
        Ok(json!({"state": if ready { "applied" } else { "waiting" }}))
    }
    /// `video.render.release`, or the owner was disabled: mpv defaults, free.
    pub fn release(&mut self, owner: u64, player: &mut impl Player) -> Result<bool, RpcError> {
        if self.owner != Some(owner) {
            return Ok(false);
        }
        self.owner = None;
        self.write(None, 60., player)?;
        Ok(true)
    }
    /// The owner holds shaders it has not been told a compile failure for
    /// (`video_shaders::Channel::compile_failure` reports to it).
    pub fn unreported_shaders(&self) -> bool {
        self.owner.is_some() && !self.reported && !self.settings.shaders.is_empty()
    }
    /// A compile failure was reported for the current settings: the owner.
    pub fn mark_reported(&mut self) -> Option<u64> {
        self.reported = true;
        self.owner
    }
    /// The shader channel's new joined list: `glsl-shaders` becomes the
    /// render options' shaders, then `list`. On a refusal the previous list
    /// is written back (best effort) and the call fails with `backend`.
    pub fn set_channel(
        &mut self,
        list: Vec<String>,
        player: &mut impl Player,
    ) -> Result<(), RpcError> {
        let own = self.owner.map_or(&[][..], |_| &self.settings.shaders[..]);
        let joined = |channel: &[String]| [own, channel].concat();
        if let Err(error) = player.shaders(&joined(&list)) {
            let _ = player.shaders(&joined(&self.channel));
            return Err(RpcError::new("backend", format!("glsl-shaders: {error}")));
        }
        self.channel = list;
        Ok(())
    }
    /// Writes every option; on a refusal the previous values are restored
    /// (best effort) and the call fails with `backend`.
    fn write(
        &mut self,
        next: Option<Settings>,
        refresh: f64,
        player: &mut impl Player,
    ) -> Result<(), RpcError> {
        let attempt = |player: &mut dyn FnMut(&str, &str) -> Result<(), String>| {
            properties(next.as_ref(), refresh)
                .iter()
                .find_map(|(name, value)| player(name, value).err().map(|e| format!("{name}: {e}")))
        };
        let channel = &self.channel;
        let joined = |settings: Option<&Settings>| {
            [settings.map_or(&[][..], |s| &s.shaders[..]), channel].concat()
        };
        let failed = attempt(&mut |name, value| player.set(name, value)).or_else(|| {
            player
                .shaders(&joined(next.as_ref()))
                .err()
                .map(|e| format!("glsl-shaders: {e}"))
        });
        if let Some(error) = failed {
            let previous = self.owner.map(|_| self.settings.clone());
            for (name, value) in properties(previous.as_ref(), refresh) {
                let _ = player.set(name, &value);
            }
            let _ = player.shaders(&joined(previous.as_ref()));
            return Err(RpcError::new("backend", error));
        }
        self.settings = next.unwrap_or_default();
        self.reported = false;
        Ok(())
    }
}

/// Whether a `libmpv_render` error line is a shader compiler's message, in
/// the formats of the compilers mpv's renderers use:
///
/// - OpenGL drivers and glslang: `ERROR: 0:39: 'x' : undeclared identifier`;
/// - shaderc (Direct3D 11, GLSL to SPIR-V): `input:42: error: 'x' :
///   undeclared identifier` (recorded on Windows, 2026-10-26);
/// - D3DCompile (the HLSL that spirv-cross made): `(42,12): error X3004: …`,
///   or mpv's own `D3DCompile failed: …`.
///
/// mpv also logs the shader source with numbered lines (`[ 42] …`), which
/// may contain any text, so those never count.
///
/// A log line of libmpv's renderer that is such a message (reported to the
/// holders of shaders, `video_shaders::Channel::compile_failure`).
pub fn compile_line(prefix: &str, level: &str, text: &str) -> bool {
    prefix == "libmpv_render" && level == "error" && compile_error(text.trim())
}

fn compile_error(text: &str) -> bool {
    !text.starts_with('[')
        && (text.starts_with("ERROR:")
            || text.contains(": error:")
            || text.contains(": error X")
            || text.starts_with("D3DCompile failed"))
}

/// `video.render.status` data: the handle's status becomes `failed`.
pub fn status_failed(code: &str, message: &str) -> Value {
    json!({"state":"failed","error":{"code":code,"message":message}})
}

/// The real player.
pub struct Mpv<'a>(pub &'a rsmpv::Mpv);
impl Player for Mpv<'_> {
    fn set(&mut self, name: &str, value: &str) -> Result<(), String> {
        self.0.set_property(name, value).map_err(|e| e.to_string())
    }
    fn shaders(&mut self, paths: &[String]) -> Result<(), String> {
        // One item per call: a path may contain the list separator.
        let run = |args: &[&str]| self.0.command(args).map_err(|e| e.to_string());
        run(&["change-list", "glsl-shaders", "clr", ""])?;
        paths
            .iter()
            .try_for_each(|path| run(&["change-list", "glsl-shaders", "append", path]))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    pub(crate) struct Fake {
        props: BTreeMap<String, String>,
        /// The last `glsl-shaders` list written.
        pub(crate) shaders: Vec<String>,
        refuse: Option<&'static str>,
    }
    impl Options {
        /// What the render owner hears for a log line (the backend asks
        /// `compile_line`, then the shader channel, which reports to it).
        fn shader_failure(
            &mut self,
            prefix: &str,
            level: &str,
            text: &str,
        ) -> Option<(u64, Value)> {
            if !compile_line(prefix, level, text) {
                return None;
            }
            crate::playback::video_shaders::Channel::default()
                .compile_failure(self, text)
                .into_iter()
                .next()
                .map(|(owner, _, status)| (owner, status))
        }
    }
    impl Player for Fake {
        fn set(&mut self, name: &str, value: &str) -> Result<(), String> {
            if self.refuse == Some(name) && value != "auto" && value != "no" && value != "audio" {
                return Err("refused".into());
            }
            self.props.insert(name.into(), value.into());
            Ok(())
        }
        fn shaders(&mut self, paths: &[String]) -> Result<(), String> {
            self.shaders = paths.to_vec();
            Ok(())
        }
    }
    fn shader() -> tempfile::NamedTempFile {
        tempfile::Builder::new().suffix(".glsl").tempfile().unwrap()
    }

    #[test]
    fn fields_map_to_mpv_options() {
        let map =
            |s: Option<&Settings>| -> BTreeMap<_, _> { properties(s, 120.).into_iter().collect() };
        let defaults = map(None);
        assert_eq!(defaults["tone-mapping"], "auto");
        assert_eq!(defaults["target-peak"], "auto");
        assert_eq!(defaults["video-sync"], "audio");
        let set = Settings::default()
            .merged(&json!({"toneMapping":"hable","targetPeak":300,"peakDetect":false,"interpolation":true}))
            .unwrap();
        let props = map(Some(&set));
        assert_eq!(props["tone-mapping"], "hable");
        assert_eq!(props["target-peak"], "300");
        assert_eq!(props["hdr-compute-peak"], "no");
        assert_eq!(props["interpolation"], "yes");
        assert_eq!(props["video-sync"], "display-resample");
        assert_eq!(props["display-fps-override"], "120");
        let off = set
            .merged(&json!({"toneMapping":"off","peakDetect":true}))
            .unwrap();
        let props = map(Some(&off));
        assert_eq!(
            (
                props["tone-mapping"].as_str(),
                props["gamut-mapping-mode"].as_str()
            ),
            ("clip", "clip")
        );
        assert_eq!(props["hdr-compute-peak"], "no", "off maps nothing");
        assert_eq!(off.target_peak, 300., "merged, not replaced");
    }

    #[test]
    fn bad_values_are_invalid() {
        let file = shader();
        let path = file.path().to_str().unwrap().to_owned();
        assert_eq!(
            Settings::default()
                .merged(&json!({"shaders":[path]}))
                .unwrap()
                .shaders,
            std::slice::from_ref(&path)
        );
        for values in [
            json!({"toneMapping":"aces"}),
            json!({"targetPeak":99}),
            json!({"targetPeak":401}),
            json!({"peakDetect":1}),
            json!({"interpolation":"yes"}),
            json!({"shaders":"a.glsl"}),
            json!({"shaders":["relative.glsl"]}),
            json!({"shaders":["/nonexistent/vestra.glsl"]}),
            json!({"shaders":vec![path.clone(); 17]}),
            json!({"scale":"ewa"}),
            json!([1]),
        ] {
            let error = Settings::default().merged(&values).unwrap_err();
            assert_eq!(error.code, "invalid", "{values}");
        }
    }

    #[test]
    fn one_owner_until_it_releases_and_mpv_defaults_come_back() {
        let (mut fake, mut options) = (Fake::default(), Options::default());
        let file = shader();
        let path = file.path().to_str().unwrap().to_owned();
        let state = options
            .set(1, &json!({"shaders":[path.clone()]}), false, 60., &mut fake)
            .unwrap();
        assert_eq!(state["state"], "waiting");
        assert_eq!(fake.shaders, std::slice::from_ref(&path));
        assert_eq!(fake.props["target-peak"], "203");
        let error = options
            .set(2, &json!({}), true, 60., &mut fake)
            .unwrap_err();
        assert_eq!(error.code, "conflict");
        assert!(!options.release(2, &mut fake).unwrap());
        // A compile error reaches the owner once.
        assert!(
            options
                .shader_failure("libmpv_render", "error", "[ 39] vec4 hook()")
                .is_none()
        );
        let (owner, status) = options
            .shader_failure("libmpv_render", "error", "ERROR: 0:39: undeclared\n")
            .unwrap();
        assert_eq!(owner, 1);
        assert_eq!(
            status["error"]["message"],
            "shader compile failed: ERROR: 0:39: undeclared"
        );
        assert!(
            options
                .shader_failure("libmpv_render", "error", "ERROR: again")
                .is_none()
        );
        assert!(options.release(1, &mut fake).unwrap());
        assert!(
            options
                .shader_failure(
                    "libmpv_render",
                    "error",
                    "input:42: error: 'x' : undeclared identifier"
                )
                .is_none(),
            "released: no owner"
        );
        assert_eq!(options.owner(), None);
        assert!(fake.shaders.is_empty());
        assert_eq!(fake.props["target-peak"], "auto");
        assert_eq!(options.settings(), &Settings::default());
        options.set(2, &json!({}), true, 60., &mut fake).unwrap();
        assert_eq!(options.owner(), Some(2), "free again");
        assert!(
            options
                .shader_failure("libmpv_render", "error", "ERROR: x")
                .is_none(),
            "no shaders, not ours"
        );
    }

    /// The compiler lines of a broken shader (`undefined_name`) as libmpv
    /// logs them: OpenGL on macOS, and on Windows (Direct3D 11) the lines
    /// recorded on the parked `gpu-render` branch (2026-10-26) and checked
    /// again on WARP by `warp_a_broken_shader_logs_a_line_that_compile_error_counts`.
    #[test]
    fn compiler_messages_of_every_renderer_count_and_the_source_listing_does_not() {
        for line in [
            "ERROR: 0:39: 'undefined_name' : undeclared identifier",
            "input:42: error: 'undefined_name' : undeclared identifier",
            "C:\\shader(42,12-25): error X3004: undeclared identifier 'undefined_name'",
            "D3DCompile failed: 0x80004005",
        ] {
            assert!(compile_error(line), "{line}");
        }
        for line in [
            "shaderc output:",
            "shaderc compile status 'error' (1 errors, 0 warnings)",
            "GLSL source:",
            "[ 42]     return HOOKED_tex(HOOKED_pos) * undefined_name;",
            "[ 43] // ERROR: 0:1: not an error, a comment : error: in the source",
            "Disabling filter vestra-owned because it has failed.",
        ] {
            assert!(!compile_error(line), "{line}");
        }
        let (mut fake, mut options) = (Fake::default(), Options::default());
        let file = shader();
        let path = file.path().to_str().unwrap().to_owned();
        options
            .set(1, &json!({"shaders":[path]}), true, 60., &mut fake)
            .unwrap();
        let (owner, status) = options
            .shader_failure(
                "libmpv_render",
                "error",
                "input:42: error: 'undefined_name' : undeclared identifier\n",
            )
            .unwrap();
        assert_eq!(owner, 1);
        assert_eq!(
            status["error"]["message"],
            "shader compile failed: input:42: error: 'undefined_name' : undeclared identifier"
        );
    }

    /// Windows: libmpv's Direct3D 11 renderer (on WARP, no GPU needed) logs
    /// a broken shader as a `libmpv_render` error that `compile_error`
    /// counts, so `video.render` reports `failed` there as on macOS.
    #[cfg(windows)]
    #[test]
    fn warp_a_broken_shader_logs_a_line_that_compile_error_counts() {
        use std::time::{Duration, Instant};
        let shader =
            std::env::temp_dir().join(format!("vestra-broken-{}.glsl", std::process::id()));
        let text = "//!HOOK MAIN\n//!BIND HOOKED\nvec4 hook() {\n    return HOOKED_tex(HOOKED_pos) * undefined_name;\n}\n";
        std::fs::write(&shader, text).unwrap();
        let mpv = std::sync::Arc::new(
            rsmpv::Mpv::builder()
                .unwrap()
                .set_property("vo", "libmpv")
                .unwrap()
                .set_property("ao", "null")
                .unwrap()
                .set_property("hwdec", "no")
                .unwrap()
                .build()
                .unwrap(),
        );
        mpv.request_log_messages("v").unwrap();
        let publish = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let renderer = vestra_gpu_render::start_on_warp(mpv.clone(), (64, 64), publish).unwrap();
        let path = shader.to_str().unwrap();
        mpv.command(&["change-list", "glsl-shaders", "append", path])
            .unwrap();
        mpv.command(&["loadfile", "av://lavfi:testsrc2=s=64x64:d=30"])
            .unwrap();
        let (handle, mut errors, start) = (renderer.handle(), Vec::new(), Instant::now());
        while start.elapsed() < Duration::from_secs(10)
            && (handle.frames() < 3 || errors.is_empty())
        {
            while let Some(event) = mpv.poll_event() {
                if let rsmpv::Event::LogMessage(message) = event
                    && message.prefix == "libmpv_render"
                    && message.level == "error"
                {
                    errors.push(message.text);
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = std::fs::remove_file(&shader);
        assert!(errors.iter().any(|text| compile_error(text)), "{errors:#?}");
        assert!(handle.frames() > 0, "mpv still renders without the shader");
    }

    #[test]
    fn a_refused_option_restores_the_previous_values() {
        let (mut fake, mut options) = (Fake::default(), Options::default());
        options
            .set(1, &json!({"interpolation":true}), true, 60., &mut fake)
            .unwrap();
        fake.refuse = Some("target-peak");
        let error = options
            .set(1, &json!({"targetPeak":350}), true, 60., &mut fake)
            .unwrap_err();
        assert_eq!(error.code, "backend");
        assert!(options.settings().interpolation);
        assert_eq!(options.settings().target_peak, 203.);
        assert_eq!(fake.props["interpolation"], "yes");
    }
}
