//! Bounded declarations and media descriptors. Site extraction stays in trusted JS.
#![forbid(unsafe_code)]
use crate::RpcError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub id: String,
    pub schemes: Vec<String>,
    pub hosts: Vec<String>,
}
#[derive(Default)]
pub struct Registry {
    entries: BTreeMap<(u64, String), Declaration>,
}
impl Registry {
    pub fn register(&mut self, owner: u64, mut declaration: Declaration) -> Result<(), RpcError> {
        if declaration.id.is_empty()
            || declaration.id.len() > 64
            || !declaration
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            || declaration.schemes.is_empty()
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
        let key = (owner, declaration.id.clone());
        if self.entries.contains_key(&key) {
            return Err(RpcError::new("conflict", "resolver id already registered"));
        }
        for existing in self.entries.values() {
            if existing
                .hosts
                .iter()
                .any(|host| declaration.hosts.contains(host))
                && existing
                    .schemes
                    .iter()
                    .any(|scheme| declaration.schemes.contains(scheme))
            {
                return Err(RpcError::new(
                    "conflict",
                    "resolver host and scheme already registered",
                ));
            }
        }
        if self.entries.len() >= 16 {
            return Err(RpcError::new("busy", "resolver registry full"));
        }
        self.entries.insert(key, declaration);
        Ok(())
    }
    pub fn find(&self, source: &str) -> Option<(u64, String)> {
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
    pub fn remove(&mut self, owner: u64, id: &str) {
        self.entries.remove(&(owner, id.into()));
    }
    pub fn disable(&mut self, owner: u64) {
        self.entries.retain(|(id, _), _| *id != owner);
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
mod tests {
    use super::*;
    #[test]
    fn exact_host_registration_conflicts_and_owner_disposal() {
        let mut r = Registry::default();
        let declaration = || Declaration {
            id: "provider".into(),
            schemes: vec!["https".into()],
            hosts: vec!["Media.Example".into()],
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
