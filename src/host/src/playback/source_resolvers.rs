//! Bounded declarations and media descriptors. Site extraction stays in trusted JS.
//!
//! Two forms of declaration: web pages (`schemes` + `hosts`, resolved to
//! media URLs) and local files (`extensions`, resolved to a stream the
//! resolver's plugin generates). One scheme + host pair, and one extension,
//! has one owner; another plugin asking for it is refused with `conflict`
//! (`resolver-taken`), which does not fail its activation.
#![forbid(unsafe_code)]
use crate::RpcError;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

/// File resolvers name at most this many extensions.
const MAX_EXTENSIONS: usize = 16;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub id: String,
    #[serde(default)]
    pub schemes: Vec<String>,
    #[serde(default)]
    pub hosts: Vec<String>,
    /// Local files by extension (lower case, without the dot).
    #[serde(default)]
    pub extensions: Vec<String>,
}
impl Declaration {
    /// A file resolver (`extensions`), which answers with a generated stream.
    pub fn files(&self) -> bool {
        !self.extensions.is_empty()
    }
}
/// The extension a file resolver matches for `source`: local absolute paths
/// only (no `://`), compared in lower case. Checked before any URL parsing,
/// because `C:\a.nes` parses as a URL with the scheme `c`.
fn file_extension(source: &str) -> Option<String> {
    extension_of(source, cfg!(windows))
}
/// [`file_extension`] by one system's path rules, so both are tested on any
/// system. Unix: absolute paths start with `/`. Windows (as
/// `Path::is_absolute` there): a drive and a separator (`C:\`, `C:/`), or a
/// UNC or verbatim prefix (`\\server\share`, `\\?\C:\`, `//server/share`);
/// both separators count. A rooted path without a drive (`\roms`) or a
/// drive-relative one (`C:roms`) is not absolute there.
fn extension_of(source: &str, windows: bool) -> Option<String> {
    if source.contains("://") {
        return None;
    }
    let separator = |c: char| c == '/' || (windows && c == '\\');
    let absolute = if windows {
        let c: Vec<char> = source.chars().take(3).collect();
        let drive = c.len() == 3 && c[0].is_ascii_alphabetic() && c[1] == ':' && separator(c[2]);
        let share = c.len() == 3 && separator(c[0]) && separator(c[1]) && !separator(c[2]);
        drive || share
    } else {
        source.starts_with('/')
    };
    if !absolute {
        return None;
    }
    let name = source.rsplit(separator).next()?;
    let dot = name.rfind('.').filter(|&i| i > 0)?;
    let extension = &name[dot + 1..];
    (!extension.is_empty()).then(|| extension.to_ascii_lowercase())
}
/// `conflict` because another plugin holds it: not an activation failure.
/// `holder` is the owner number; the main thread names the plugin.
fn taken(holder: u64, what: String) -> RpcError {
    RpcError::taken(
        "resolver-taken",
        crate::i18n::tr_args("{} is resolved by {}", &json!([what, holder])),
        json!({ "holder": holder, "target": what }),
    )
}
#[derive(Default)]
pub struct Registry {
    entries: BTreeMap<(u64, String), Declaration>,
}
impl Registry {
    fn validate_id(id: &str) -> bool {
        !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
    }
    fn validate_files(declaration: &mut Declaration) -> Result<(), RpcError> {
        if !declaration.schemes.is_empty() || !declaration.hosts.is_empty() {
            return Err(RpcError::invalid(
                "a resolver declares either schemes and hosts or extensions",
            ));
        }
        let mut extensions = crate::ui::drops::parse_extensions(&json!(declaration.extensions))
            .map_err(RpcError::invalid)?;
        if extensions.len() > MAX_EXTENSIONS || extensions.iter().any(|e| e == "folder") {
            return Err(RpcError::invalid(
                "file resolvers take 1–16 file extensions (not folder)",
            ));
        }
        extensions.sort();
        extensions.dedup();
        declaration.extensions = extensions;
        Ok(())
    }
    fn validate_web(declaration: &mut Declaration) -> Result<(), RpcError> {
        if declaration.schemes.is_empty()
            || declaration.schemes.len() > 2
            || declaration.hosts.is_empty()
            || declaration.hosts.len() > 16
        {
            return Err(RpcError::invalid(
                "bounded resolver id, schemes and hosts required",
            ));
        }
        declaration.schemes.sort();
        declaration.schemes.dedup();
        if declaration
            .schemes
            .iter()
            .any(|s| !matches!(s.as_str(), "http" | "https"))
        {
            return Err(RpcError::invalid("resolver schemes support http and https"));
        }
        for host in &mut declaration.hosts {
            *host = host.to_ascii_lowercase();
            let parsed = url::Url::parse(&format!("https://{host}/"))
                .map_err(|_| RpcError::invalid("invalid resolver host"))?;
            if host.len() > 253
                || parsed.host_str() != Some(host.as_str())
                || parsed.port().is_some()
                || !parsed.username().is_empty()
                || parsed.password().is_some()
                || parsed.path() != "/"
            {
                return Err(RpcError::invalid(
                    "resolver hosts must be exact normalized host names",
                ));
            }
        }
        declaration.hosts.sort();
        declaration.hosts.dedup();
        Ok(())
    }
    /// The registration already holding part of `declaration`, and what.
    fn holder(&self, declaration: &Declaration) -> Option<(u64, String)> {
        self.entries.iter().find_map(|((owner, _), existing)| {
            if let Some(extension) = existing
                .extensions
                .iter()
                .find(|e| declaration.extensions.contains(e))
            {
                return Some((
                    *owner,
                    crate::i18n::tr_args("File type .{}", &json!([extension])),
                ));
            }
            let host = existing
                .hosts
                .iter()
                .find(|host| declaration.hosts.contains(host))?;
            let scheme = existing
                .schemes
                .iter()
                .find(|scheme| declaration.schemes.contains(scheme))?;
            Some((*owner, format!("{scheme}://{host}")))
        })
    }
    pub fn register(&mut self, owner: u64, mut declaration: Declaration) -> Result<(), RpcError> {
        if !Self::validate_id(&declaration.id) {
            return Err(RpcError::invalid(
                "bounded resolver id, schemes and hosts required",
            ));
        }
        if declaration.files() {
            Self::validate_files(&mut declaration)?;
        } else {
            Self::validate_web(&mut declaration)?;
        }
        let key = (owner, declaration.id.clone());
        if self.entries.contains_key(&key) {
            return Err(RpcError::new("conflict", "resolver id already registered"));
        }
        if let Some((holder, what)) = self.holder(&declaration) {
            // The plugin's own other registration: a plain conflict (its mistake).
            if holder == owner {
                return Err(RpcError::new(
                    "conflict",
                    format!("{what} already registered by this plugin"),
                ));
            }
            return Err(taken(holder, what));
        }
        if self.entries.len() >= 16 {
            return Err(RpcError::new("busy", "resolver registry full"));
        }
        self.entries.insert(key, declaration);
        Ok(())
    }
    pub fn find(&self, source: &str) -> Option<(u64, String)> {
        if let Some(extension) = file_extension(source) {
            return self
                .entries
                .iter()
                .find(|(_, d)| d.extensions.contains(&extension))
                .map(|(key, _)| key.clone());
        }
        let parsed = url::Url::parse(source).ok()?;
        let host = parsed.host_str()?;
        self.entries
            .iter()
            .find(|(_, d)| {
                d.schemes.iter().any(|s| s == parsed.scheme()) && d.hosts.iter().any(|h| h == host)
            })
            .map(|(key, _)| key.clone())
    }
    pub fn contains(&self, owner: u64, id: &str) -> bool {
        self.entries.contains_key(&(owner, id.into()))
    }
    /// Whether this registration is a file resolver (answers with streams).
    pub fn files(&self, owner: u64, id: &str) -> bool {
        self.entries
            .get(&(owner, id.into()))
            .is_some_and(Declaration::files)
    }
    pub fn remove(&mut self, owner: u64, id: &str) {
        self.entries.remove(&(owner, id.into()));
    }
    pub fn disable(&mut self, owner: u64) {
        self.entries.retain(|(id, _), _| *id != owner);
    }
    /// `plugin.resolvers.fileTypes`: every registered file resolver's
    /// extensions, lower case, sorted, once each (not who registered them).
    pub fn file_types(&self) -> Vec<String> {
        let mut types = self
            .entries
            .values()
            .flat_map(|d| d.extensions.iter().cloned())
            .collect::<Vec<_>>();
        types.sort();
        types.dedup();
        types
    }
}
/// What a file resolver answers: the producer it prepared (`producer.prepare`).
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Stream {
    pub kind: String,
    pub producer_id: u64,
}
impl Stream {
    pub fn producer(&self) -> Result<u64, String> {
        if self.kind != "stream" || self.producer_id == 0 {
            return Err("invalid resolved stream descriptor".into());
        }
        Ok(self.producer_id)
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Media {
    pub kind: String,
    pub original_url: String,
    pub title: Option<String>,
    pub primary: Resource,
    pub audio: Option<Resource>,
}
impl Resource {
    fn validate(&mut self) -> Result<(), String> {
        let url = url::Url::parse(&self.url).map_err(|_| "invalid resource URL")?;
        if self.url.len() > 16384 || !matches!(url.scheme(), "http" | "https") {
            return Err("resolved resources require bounded HTTP(S) URLs".into());
        }
        if self.headers.len() > 32 {
            return Err("resource header limit exceeded".into());
        }
        let mut normalized = BTreeMap::new();
        let mut bytes = 0;
        for (name, value) in &self.headers {
            bytes += name.len() + value.len();
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
                || value.contains(['\r', '\n', '\0'])
                || bytes > 16384
                || normalized
                    .insert(name.to_ascii_lowercase(), value.clone())
                    .is_some()
            {
                return Err("invalid, duplicate or oversized resource headers".into());
            }
        }
        self.headers = normalized;
        Ok(())
    }
}
impl Media {
    /// This backend supports one header set for the paired streams. Never drop a
    /// different audio header map or mutate global mpv options as a workaround.
    pub fn playback(mut self, original: &str) -> Result<(String, String), String> {
        if self.kind != "media"
            || self.original_url != original
            || self.title.as_ref().is_some_and(|s| s.len() > 1024)
        {
            return Err("resolved descriptor identity or title invalid".into());
        }
        self.primary.validate()?;
        let source = if let Some(audio) = &mut self.audio {
            audio.validate()?;
            if audio.headers != self.primary.headers {
                return Err(
                    "unsupported: paired resources require identical headers on this backend"
                        .into(),
                );
            }
            format!(
                "edl://!no_clip;!no_chapters;%{}%{};!new_stream;!no_clip;%{}%{};",
                self.primary.url.len(),
                self.primary.url,
                audio.url.len(),
                audio.url
            )
        } else {
            self.primary.url
        };
        let headers = self
            .primary
            .headers
            .iter()
            .map(|(name, value)| {
                format!("{name}: {value}")
                    .replace('\\', "\\\\")
                    .replace(',', "\\,")
            })
            .collect::<Vec<_>>()
            .join(",");
        let options = if headers.is_empty() {
            String::new()
        } else {
            format!(",http-header-fields=%{}%{}", headers.len(), headers)
        };
        Ok((source, options))
    }
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[test]
    fn exact_host_registration_conflicts_and_owner_disposal() {
        let mut r = Registry::default();
        let declaration = || Declaration {
            id: "provider".into(),
            schemes: vec!["https".into()],
            hosts: vec!["Media.Example".into()],
            extensions: vec![],
        };
        r.register(1, declaration()).unwrap();
        assert_eq!(
            r.find("https://media.example/watch?a=1"),
            Some((1, "provider".into()))
        );
        assert!(r.find("https://media.example.attacker.test/").is_none());
        assert!(r.register(2, declaration()).is_err());
        r.disable(1);
        r.register(2, declaration()).unwrap();
    }
    /// An absolute path on this system (`/…` or `C:\…`).
    pub(crate) fn rom(name: &str) -> String {
        std::env::temp_dir()
            .join("roms")
            .join(name)
            .to_string_lossy()
            .into_owned()
    }
    fn files(id: &str, extensions: &[&str]) -> Declaration {
        Declaration {
            id: id.into(),
            schemes: vec![],
            hosts: vec![],
            extensions: extensions.iter().map(|e| (*e).into()).collect(),
        }
    }
    fn reason(error: &RpcError) -> Option<&str> {
        error.detail.as_ref()?.get("reason")?.as_str()
    }
    #[test]
    fn file_resolvers_match_local_absolute_paths_by_extension() {
        let mut r = Registry::default();
        r.register(1, files("nes", &["nes", "fds", "nes"])).unwrap();
        assert!(r.files(1, "nes"));
        assert_eq!(r.find(&rom("Mario.NES")), Some((1, "nes".into())));
        assert_eq!(r.find(&rom("disk.fds")), Some((1, "nes".into())));
        // Relative paths, URLs and other extensions are not files of it.
        assert!(r.find("roms/a.nes").is_none());
        assert!(r.find("file:///roms/a.nes").is_none());
        assert!(r.find("https://example.com/a.nes").is_none());
        assert!(r.find(&rom("a.nes.zip")).is_none());
        assert!(r.find(&rom("nes")).is_none());
        // A web resolver never matches a path, a file resolver never a URL.
        r.register(
            1,
            Declaration {
                id: "web".into(),
                schemes: vec!["https".into()],
                hosts: vec!["media.example".into()],
                extensions: vec![],
            },
        )
        .unwrap();
        assert!(!r.files(1, "web"));
        assert_eq!(
            r.find("https://media.example/a.nes"),
            Some((1, "web".into()))
        );
    }
    #[test]
    fn file_types_list_every_file_resolver_once_and_go_with_their_owner() {
        let mut r = Registry::default();
        assert!(r.file_types().is_empty());
        r.register(1, files("nes", &["nes", "fds"])).unwrap();
        r.register(2, files("gb", &["gb", "gbc"])).unwrap();
        r.register(
            3,
            Declaration {
                id: "web".into(),
                schemes: vec!["https".into()],
                hosts: vec!["media.example".into()],
                extensions: vec![],
            },
        )
        .unwrap();
        assert_eq!(r.file_types(), ["fds", "gb", "gbc", "nes"]);
        r.disable(1);
        assert_eq!(r.file_types(), ["gb", "gbc"]);
        r.remove(2, "gb");
        assert!(r.file_types().is_empty());
    }

    #[test]
    fn unix_paths_are_absolute_from_the_root() {
        let ext = |s| extension_of(s, false);
        assert_eq!(ext("/roms/Game.NES").as_deref(), Some("nes"));
        assert_eq!(ext("/roms.d/game.nes").as_deref(), Some("nes"));
        for no in [
            "roms/a.nes",
            "./a.nes",
            "/roms/nes",
            "/roms/.nes",
            "/roms/a.",
            "/roms/a.nes/",
            "file:///roms/a.nes",
            "C:\\roms\\a.nes", // a relative file name on Unix
        ] {
            assert_eq!(ext(no), None, "{no}");
        }
        // A backslash is part of a Unix file name.
        assert_eq!(ext("/roms/a\\b.nes").as_deref(), Some("nes"));
    }
    #[test]
    fn windows_paths_need_a_drive_or_a_share() {
        let ext = |s| extension_of(s, true);
        for yes in [
            "C:\\roms\\Game.NES",
            "c:/roms/game.nes",
            "D:\\roms/mixed\\game.nes",
            "\\\\server\\share\\game.nes",
            "//server/share/game.nes",
            "\\\\?\\C:\\roms\\game.nes",
            "\\\\?\\UNC\\server\\share\\game.nes",
        ] {
            assert_eq!(ext(yes).as_deref(), Some("nes"), "{yes}");
        }
        for no in [
            "\\roms\\game.nes", // rooted, no drive
            "/roms/game.nes",
            "C:roms\\game.nes", // drive-relative
            "roms\\game.nes",
            "C:\\roms\\nes",
            "C:\\roms\\game.nes\\",
            "file:///C:/roms/game.nes",
        ] {
            assert_eq!(ext(no), None, "{no}");
        }
    }
    #[test]
    fn file_declarations_are_bounded_and_single_form() {
        let mut r = Registry::default();
        for bad in [
            files("a", &["NES"]),
            files("a", &[".nes"]),
            files("a", &["folder"]),
            files("a", &[""]),
            files("a", &["a"; 17]),
            Declaration {
                schemes: vec!["https".into()],
                hosts: vec!["media.example".into()],
                ..files("a", &["nes"])
            },
        ] {
            let error = r.register(1, bad).unwrap_err();
            assert_eq!(error.code, "invalid", "{}", error.message);
        }
        let error = r.register(1, files("a", &[])).unwrap_err();
        assert_eq!(error.code, "invalid");
    }
    #[test]
    fn another_plugins_extension_or_host_is_taken_its_own_is_a_plain_conflict() {
        let mut r = Registry::default();
        r.register(1, files("nes", &["nes"])).unwrap();
        let error = r.register(2, files("emu", &["gb", "nes"])).unwrap_err();
        assert_eq!(error.code, "conflict");
        assert_eq!(reason(&error), Some("resolver-taken"));
        assert_eq!(error.detail.as_ref().unwrap()["holder"], 1);
        assert_eq!(error.detail.as_ref().unwrap()["target"], "File type .nes");
        assert!(error.is_taken());
        let own = r.register(1, files("other", &["nes"])).unwrap_err();
        assert_eq!(own.code, "conflict");
        assert_eq!(reason(&own), None);
        let web = || Declaration {
            id: "web".into(),
            schemes: vec!["https".into()],
            hosts: vec!["media.example".into()],
            extensions: vec![],
        };
        r.register(1, web()).unwrap();
        let error = r.register(2, web()).unwrap_err();
        assert_eq!(reason(&error), Some("resolver-taken"));
        assert_eq!(
            error.detail.as_ref().unwrap()["target"],
            "https://media.example"
        );
        // The holder stops: the extension is free again.
        r.disable(1);
        r.register(2, files("emu", &["gb", "nes"])).unwrap();
        assert_eq!(r.find(&rom("a.nes")), Some((2, "emu".into())));
    }
    #[test]
    fn split_streams_preserve_headers_and_reject_different_audio_headers() {
        let resource = || Resource {
            url: "https://cdn.example/a,b".into(),
            headers: BTreeMap::from([("X-Test".into(), "a,b".into())]),
        };
        let media = || Media {
            kind: "media".into(),
            original_url: "https://media.example/watch".into(),
            title: None,
            primary: resource(),
            audio: Some(resource()),
        };
        let (url, options) = media().playback("https://media.example/watch").unwrap();
        assert!(url.contains("!new_stream"));
        assert!(options.contains("a\\,b"));
        let mut changed = media();
        changed.audio.as_mut().unwrap().headers.clear();
        assert!(changed.playback("https://media.example/watch").is_err());
    }
}
