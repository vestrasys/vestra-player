//! The shader channel `plugin.video.shaders(id)` (SDK 1.5.39, player
//! 0.5.20): any number of plugins each add their own mpv user shaders.
//!
//! Every handle (plugin, id) holds an ordered list of shader files. The
//! player joins the lists into mpv's `glsl-shaders` after the render
//! options' own `shaders` (`plugin.video.render()`, single owner), in a
//! stable order: built-in plugins first, then installed plugins in the
//! order they were first installed, then development loads (`--plugin`);
//! ties by manifest id, and a plugin's handles by id. At most
//! [`MAX_SHADERS`] across every handle (`limit` beyond); the render
//! options keep their own 16. Files were checked on the main thread
//! (`plugin_files::shader`, the capability layer), which also sends the
//! rank. GPU rendering only: under software rendering a non-empty list is
//! `unsupported`. Shaders act at render time, so snapshots and exports
//! never contain them. A stopped plugin's lists go (`disable`).
#![forbid(unsafe_code)]

use crate::host::RpcError;
use crate::playback::video_render::{Options, Player, status_failed};
use serde_json::{Value, json};
use std::collections::BTreeMap;

/// Shaders across every plugin's handles.
pub const MAX_SHADERS: usize = 16;
/// Bytes of a handle id.
pub const MAX_ID: usize = 128;

/// Where a handle's list goes in the joined list.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    /// 0 built-in, 1 installed, 2 development load.
    tier: u8,
    /// Unix ms of the first install (0 for the other tiers).
    installed_at: u64,
    manifest: String,
    owner: u64,
    id: String,
}

#[derive(Debug)]
struct Entry {
    paths: Vec<String>,
    /// A compile failure was pushed for the current list.
    reported: bool,
}

/// Every plugin's shader lists.
#[derive(Debug, Default)]
pub struct Channel {
    entries: BTreeMap<Key, Entry>,
}

/// `video.shaders.status` data for handle `id`.
fn status(id: &str, code: &str, message: &str) -> Value {
    let mut status = status_failed(code, message);
    status["id"] = id.into();
    status
}

impl Channel {
    fn joined(entries: &BTreeMap<Key, Entry>) -> Vec<String> {
        entries.values().flat_map(|e| e.paths.clone()).collect()
    }
    /// The joined list (after the render options' shaders in mpv).
    pub fn paths(&self) -> Vec<String> {
        Self::joined(&self.entries)
    }
    pub fn holds(&self, owner: u64) -> bool {
        self.entries.keys().any(|key| key.owner == owner)
    }
    /// Owners and ids of the handles with shaders.
    fn handles(&self) -> impl Iterator<Item = (u64, &str)> {
        self.entries.keys().map(|key| (key.owner, key.id.as_str()))
    }
    /// `video.shaders.set { id, paths, manifestId, rank }` → `{ state }`.
    /// The capability layer checked `paths` and set `manifestId` and `rank`
    /// (`[tier, installedAt]`). An empty list removes the handle's shaders.
    pub fn set(
        &mut self,
        owner: u64,
        params: &Value,
        ready: bool,
        gpu: bool,
        render: &mut Options,
        player: &mut impl Player,
    ) -> Result<Value, RpcError> {
        let id = params["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= MAX_ID && !id.contains(char::is_control))
            .ok_or_else(|| RpcError::invalid(format!("id must be 1 to {MAX_ID} bytes")))?;
        let paths = params["paths"]
            .as_array()
            .and_then(|list| {
                list.iter()
                    .map(|p| p.as_str().map(str::to_owned))
                    .collect::<Option<Vec<_>>>()
            })
            .ok_or_else(|| RpcError::invalid("paths must be an array of paths"))?;
        let rank = &params["rank"];
        let key = Key {
            tier: rank[0]
                .as_u64()
                .and_then(|t| u8::try_from(t).ok())
                .unwrap_or(2),
            installed_at: rank[1].as_u64().unwrap_or(0),
            manifest: params["manifestId"].as_str().unwrap_or_default().to_owned(),
            owner,
            id: id.to_owned(),
        };
        if !paths.is_empty() && !gpu {
            return Err(RpcError::new(
                "unsupported",
                "shaders need GPU rendering; the player renders in software",
            ));
        }
        let mut next: BTreeMap<Key, Entry> = self
            .entries
            .iter()
            .filter(|(k, _)| **k != key)
            .map(|(k, e)| {
                (
                    k.clone(),
                    Entry {
                        paths: e.paths.clone(),
                        reported: e.reported,
                    },
                )
            })
            .collect();
        let others: usize = next.values().map(|e| e.paths.len()).sum();
        if others + paths.len() > MAX_SHADERS {
            return Err(RpcError::new(
                "limit",
                format!(
                    "at most {MAX_SHADERS} shaders across all plugins: {others} are in use, this list has {}",
                    paths.len()
                ),
            ));
        }
        if !paths.is_empty() {
            next.insert(
                key,
                Entry {
                    paths,
                    reported: false,
                },
            );
        }
        let joined = Self::joined(&next);
        if joined != self.paths() {
            render.set_channel(joined, player)?;
        }
        self.entries = next;
        Ok(json!({"state": if ready { "applied" } else { "waiting" }}))
    }
    /// The owner stopped (or was parked): its shaders go. Errors only when
    /// mpv refused the new list.
    pub fn disable(
        &mut self,
        owner: u64,
        render: &mut Options,
        player: &mut impl Player,
    ) -> Result<(), RpcError> {
        if !self.holds(owner) {
            return Ok(());
        }
        self.entries.retain(|key, _| key.owner != owner);
        render.set_channel(self.paths(), player)
    }
    /// The player switched to software rendering: every handle with
    /// shaders hears `unsupported`.
    pub fn software_fallback(&self) -> Vec<(u64, Value)> {
        self.handles()
            .map(|(owner, id)| {
                (
                    owner,
                    status(
                        id,
                        "unsupported",
                        "the player switched to software rendering",
                    ),
                )
            })
            .collect()
    }
    /// A shader mpv could not compile (`video_render::compile_line`).
    /// mpv's message does not say which file failed, so every holder of
    /// shaders not yet told hears it: the render options' owner and each
    /// handle, once per list. With one holder the message is mpv's line,
    /// as before; with several it says the player cannot tell whose.
    /// Returns `(owner, event, data)`.
    pub fn compile_failure(
        &mut self,
        render: &mut Options,
        text: &str,
    ) -> Vec<(u64, &'static str, Value)> {
        let render_holds = render.unreported_shaders();
        let holders = usize::from(render_holds)
            + self
                .entries
                .values()
                .filter(|e| !e.reported && !e.paths.is_empty())
                .count();
        if holders == 0 {
            return Vec::new();
        }
        let text = text.trim();
        let message = if holders == 1 {
            format!("shader compile failed: {text}")
        } else {
            format!(
                "shader compile failed (shaders of {holders} handles are on; the player cannot tell whose failed): {text}"
            )
        };
        let mut pushes = Vec::new();
        if render_holds && let Some(owner) = render.mark_reported() {
            pushes.push((
                owner,
                "video.render.status",
                status_failed("failed", &message),
            ));
        }
        for (key, entry) in &mut self.entries {
            if !entry.reported && !entry.paths.is_empty() {
                entry.reported = true;
                pushes.push((
                    key.owner,
                    "video.shaders.status",
                    status(&key.id, "failed", &message),
                ));
            }
        }
        pushes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playback::video_render::tests::Fake;

    fn set(
        channel: &mut Channel,
        render: &mut Options,
        fake: &mut Fake,
        owner: u64,
        (manifest, rank): (&str, [u64; 2]),
        id: &str,
        paths: &[&str],
    ) -> Result<Value, RpcError> {
        channel.set(
            owner,
            &json!({"id":id,"paths":paths,"manifestId":manifest,"rank":rank}),
            true,
            true,
            render,
            fake,
        )
    }

    #[test]
    fn lists_join_built_ins_first_then_install_order_then_development() {
        let (mut channel, mut render, mut fake) =
            (Channel::default(), Options::default(), Fake::default());
        let dev = ("zz.dev", [2, 0]);
        let late = ("aa.late", [1, 200]);
        let early = ("zz.early", [1, 100]);
        let builtin = ("vestra.video", [0, 0]);
        set(&mut channel, &mut render, &mut fake, 4, dev, "a", &["/d"]).unwrap();
        set(&mut channel, &mut render, &mut fake, 3, late, "a", &["/l"]).unwrap();
        set(
            &mut channel,
            &mut render,
            &mut fake,
            2,
            early,
            "b",
            &["/e2"],
        )
        .unwrap();
        set(
            &mut channel,
            &mut render,
            &mut fake,
            2,
            early,
            "a",
            &["/e1", "/e1b"],
        )
        .unwrap();
        set(
            &mut channel,
            &mut render,
            &mut fake,
            1,
            builtin,
            "x",
            &["/b"],
        )
        .unwrap();
        assert_eq!(fake.shaders, ["/b", "/e1", "/e1b", "/e2", "/l", "/d"]);
        // The render options' own shaders come first.
        let file = tempfile::Builder::new().suffix(".glsl").tempfile().unwrap();
        let path = file.path().to_str().unwrap().to_owned();
        render
            .set(9, &json!({"shaders":[path.clone()]}), true, 60., &mut fake)
            .unwrap();
        assert_eq!(fake.shaders[0], path);
        assert_eq!(fake.shaders[1..], ["/b", "/e1", "/e1b", "/e2", "/l", "/d"]);
        render.release(9, &mut fake).unwrap();
        assert_eq!(fake.shaders, ["/b", "/e1", "/e1b", "/e2", "/l", "/d"]);
        // Replacing a list keeps its place; an empty one removes it.
        set(&mut channel, &mut render, &mut fake, 3, late, "a", &["/l2"]).unwrap();
        set(&mut channel, &mut render, &mut fake, 2, early, "a", &[]).unwrap();
        assert_eq!(fake.shaders, ["/b", "/e2", "/l2", "/d"]);
    }

    #[test]
    fn at_most_sixteen_across_all_plugins() {
        let (mut channel, mut render, mut fake) =
            (Channel::default(), Options::default(), Fake::default());
        let ten: Vec<String> = (0..10).map(|i| format!("/a{i}")).collect();
        let ten: Vec<&str> = ten.iter().map(String::as_str).collect();
        set(
            &mut channel,
            &mut render,
            &mut fake,
            1,
            ("a", [1, 1]),
            "x",
            &ten,
        )
        .unwrap();
        let seven = ["/b"; 7];
        let error = set(
            &mut channel,
            &mut render,
            &mut fake,
            2,
            ("b", [1, 2]),
            "x",
            &seven,
        )
        .unwrap_err();
        assert_eq!(error.code, "limit");
        assert!(error.message.contains("10 are in use"), "{}", error.message);
        assert_eq!(fake.shaders.len(), 10, "nothing changed");
        set(
            &mut channel,
            &mut render,
            &mut fake,
            2,
            ("b", [1, 2]),
            "x",
            &seven[..6],
        )
        .unwrap();
        assert_eq!(fake.shaders.len(), 16);
        // A handle's own list does not count against itself.
        set(
            &mut channel,
            &mut render,
            &mut fake,
            1,
            ("a", [1, 1]),
            "x",
            &ten[..10],
        )
        .unwrap();
        for bad in [
            json!({"id":"","paths":[]}),
            json!({"id":"x".repeat(129),"paths":[]}),
            json!({"id":"x","paths":"/a"}),
            json!({"id":"x","paths":[1]}),
        ] {
            let error = channel
                .set(1, &bad, true, true, &mut render, &mut fake)
                .unwrap_err();
            assert_eq!(error.code, "invalid", "{bad}");
        }
    }

    #[test]
    fn a_stopped_plugin_loses_only_its_own_shaders() {
        let (mut channel, mut render, mut fake) =
            (Channel::default(), Options::default(), Fake::default());
        set(
            &mut channel,
            &mut render,
            &mut fake,
            1,
            ("a", [1, 1]),
            "x",
            &["/a"],
        )
        .unwrap();
        set(
            &mut channel,
            &mut render,
            &mut fake,
            1,
            ("a", [1, 1]),
            "y",
            &["/a2"],
        )
        .unwrap();
        set(
            &mut channel,
            &mut render,
            &mut fake,
            2,
            ("b", [1, 2]),
            "x",
            &["/b"],
        )
        .unwrap();
        assert!(channel.holds(1));
        channel.disable(1, &mut render, &mut fake).unwrap();
        assert!(!channel.holds(1));
        assert_eq!(fake.shaders, ["/b"]);
        channel.disable(2, &mut render, &mut fake).unwrap();
        assert!(fake.shaders.is_empty());
    }

    #[test]
    fn software_rendering_refuses_shaders_but_accepts_clearing() {
        let (mut channel, mut render, mut fake) =
            (Channel::default(), Options::default(), Fake::default());
        let params = json!({"id":"x","paths":["/a"],"manifestId":"a","rank":[1,1]});
        let error = channel
            .set(1, &params, true, false, &mut render, &mut fake)
            .unwrap_err();
        assert_eq!(error.code, "unsupported");
        let clear = json!({"id":"x","paths":[],"manifestId":"a","rank":[1,1]});
        let state = channel
            .set(1, &clear, false, false, &mut render, &mut fake)
            .unwrap();
        assert_eq!(state["state"], "waiting");
        set(
            &mut channel,
            &mut render,
            &mut fake,
            1,
            ("a", [1, 1]),
            "x",
            &["/a"],
        )
        .unwrap();
        let pushes = channel.software_fallback();
        assert_eq!(pushes.len(), 1);
        assert_eq!(pushes[0].1["error"]["code"], "unsupported");
        assert_eq!(pushes[0].1["id"], "x");
    }

    #[test]
    fn a_compile_failure_reaches_every_holder_once_and_says_when_it_is_ambiguous() {
        let (mut channel, mut render, mut fake) =
            (Channel::default(), Options::default(), Fake::default());
        set(
            &mut channel,
            &mut render,
            &mut fake,
            1,
            ("a", [1, 1]),
            "x",
            &["/a"],
        )
        .unwrap();
        let pushes = channel.compile_failure(&mut render, "ERROR: 0:1: x\n");
        assert_eq!(pushes.len(), 1);
        assert_eq!((pushes[0].0, pushes[0].1), (1, "video.shaders.status"));
        assert_eq!(
            pushes[0].2["error"]["message"],
            "shader compile failed: ERROR: 0:1: x"
        );
        assert!(
            channel
                .compile_failure(&mut render, "ERROR: again")
                .is_empty()
        );
        // Two holders: both hear it, and the message says it is ambiguous.
        set(
            &mut channel,
            &mut render,
            &mut fake,
            2,
            ("b", [1, 2]),
            "x",
            &["/b"],
        )
        .unwrap();
        set(
            &mut channel,
            &mut render,
            &mut fake,
            1,
            ("a", [1, 1]),
            "x",
            &["/a2"],
        )
        .unwrap();
        let file = tempfile::Builder::new().suffix(".glsl").tempfile().unwrap();
        let path = file.path().to_str().unwrap().to_owned();
        render
            .set(9, &json!({"shaders":[path]}), true, 60., &mut fake)
            .unwrap();
        let pushes = channel.compile_failure(&mut render, "ERROR: y");
        let owners: Vec<_> = pushes
            .iter()
            .map(|(owner, event, _)| (*owner, *event))
            .collect();
        assert_eq!(
            owners,
            [
                (9, "video.render.status"),
                (1, "video.shaders.status"),
                (2, "video.shaders.status")
            ]
        );
        assert!(
            pushes[0].2["error"]["message"]
                .as_str()
                .unwrap()
                .contains("shaders of 3 handles are on")
        );
    }
}
