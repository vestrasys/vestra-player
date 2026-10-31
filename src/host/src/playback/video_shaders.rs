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
//! `unsupported`, and a switch to software clears every list (each handle
//! hears `unsupported`). Shaders act at render time, so snapshots and exports
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
    /// What mpv's `glsl-shaders` holds after the render options' shaders.
    entries: BTreeMap<Key, Entry>,
    /// Owners whose lists must go (stopped, or a switch to software
    /// rendering) but which mpv still holds because it refused the shorter
    /// list: `entries` keeps them, so it stays what mpv has, and every
    /// later write (`flush`, `set`) leaves them out again.
    stale: std::collections::BTreeSet<u64>,
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
        !self.stale.contains(&owner) && self.entries.keys().any(|key| key.owner == owner)
    }
    /// Owners and ids of the live handles with shaders.
    fn handles(&self) -> impl Iterator<Item = (u64, &str)> {
        self.entries
            .keys()
            .filter(|key| !self.stale.contains(&key.owner))
            .map(|key| (key.owner, key.id.as_str()))
    }
    /// The entries without the stale owners' (and without `skip`).
    fn live(&self, skip: Option<&Key>) -> BTreeMap<Key, Entry> {
        self.entries
            .iter()
            .filter(|(k, _)| !self.stale.contains(&k.owner) && Some(*k) != skip)
            .map(|(k, e)| {
                (
                    k.clone(),
                    Entry {
                        paths: e.paths.clone(),
                        reported: e.reported,
                    },
                )
            })
            .collect()
    }
    /// Write the list without the stale owners' shaders. On a refusal
    /// `entries` keeps them (mpv does: `set_channel` restored the old
    /// list) and the next write tries again.
    fn flush(&mut self, render: &mut Options, player: &mut impl Player) -> Result<(), RpcError> {
        if self.stale.is_empty() {
            return Ok(());
        }
        let next = self.live(None);
        let joined = Self::joined(&next);
        if joined != self.paths() {
            render.set_channel(joined, player)?;
        }
        self.entries = next;
        self.stale.clear();
        Ok(())
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
        let mut next = self.live(Some(&key));
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
        // The stale owners' shaders went with this write.
        self.stale.clear();
        Ok(json!({"state": if ready { "applied" } else { "waiting" }}))
    }
    /// The owner stopped (or was parked): its shaders go. Errors only when
    /// mpv refused the new list; then mpv and `entries` both keep them and
    /// the next write removes them (`flush`).
    pub fn disable(
        &mut self,
        owner: u64,
        render: &mut Options,
        player: &mut impl Player,
    ) -> Result<(), RpcError> {
        if self.entries.keys().any(|key| key.owner == owner) {
            self.stale.insert(owner);
        }
        self.flush(render, player)
    }
    /// The player switched to software rendering: every handle with
    /// shaders hears `unsupported` and its list goes (it no longer counts
    /// against the limit or keeps the plugin resident); a plugin sets it
    /// again after the player draws on the GPU again (a restart). Returns
    /// the pushes, and an error when mpv refused the shorter list (it is
    /// tried again at the next write).
    pub fn software_fallback(
        &mut self,
        render: &mut Options,
        player: &mut impl Player,
    ) -> (Vec<(u64, Value)>, Result<(), RpcError>) {
        let pushes: Vec<_> = self
            .handles()
            .map(|(owner, id)| {
                (
                    owner,
                    status(
                        id,
                        "unsupported",
                        "the player switched to software rendering; set the shaders again once it draws on the GPU",
                    ),
                )
            })
            .collect();
        let owners: Vec<u64> = self.entries.keys().map(|key| key.owner).collect();
        self.stale.extend(owners);
        (pushes, self.flush(render, player))
    }
    /// A shader mpv could not compile (`video_render::compile_line`).
    /// mpv's message does not say which file failed. The message form
    /// comes from everyone whose shaders are on now (the render options'
    /// owner and each live handle): with one, mpv's line as before; with
    /// several, it says the player cannot tell whose failed (also when the
    /// others were told already: a good list joining a broken one must not
    /// be blamed with certainty). Only holders not yet told for their
    /// current list hear it. Returns `(owner, event, data)`.
    pub fn compile_failure(
        &mut self,
        render: &mut Options,
        text: &str,
    ) -> Vec<(u64, &'static str, Value)> {
        let stale = &self.stale;
        let live =
            |key: &Key, entry: &Entry| !stale.contains(&key.owner) && !entry.paths.is_empty();
        let holders = usize::from(render.holds_shaders())
            + self.entries.iter().filter(|(k, e)| live(k, e)).count();
        let render_holds = render.unreported_shaders();
        let untold = self.entries.iter().any(|(k, e)| live(k, e) && !e.reported);
        if !render_holds && !untold {
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
            if !entry.reported && !entry.paths.is_empty() && !self.stale.contains(&key.owner) {
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
        set(
            &mut channel,
            &mut render,
            &mut fake,
            2,
            ("b", [1, 2]),
            "y",
            &["/b"],
        )
        .unwrap();
        let (pushes, written) = channel.software_fallback(&mut render, &mut fake);
        written.unwrap();
        assert_eq!(pushes.len(), 2, "every handle hears it");
        assert_eq!(pushes[0].1["error"]["code"], "unsupported");
        assert_eq!(pushes[0].1["id"], "x");
        // The lists went: they no longer count or keep the plugins live.
        assert!(fake.shaders.is_empty());
        assert!(!channel.holds(1) && !channel.holds(2));
        assert!(channel.paths().is_empty());
        let state = channel
            .set(1, &clear, true, false, &mut render, &mut fake)
            .unwrap();
        assert_eq!(state["state"], "applied", "clear() still succeeds");
    }

    #[test]
    fn a_refused_removal_keeps_channel_and_mpv_alike_and_is_retried() {
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
            2,
            ("b", [1, 2]),
            "x",
            &["/b"],
        )
        .unwrap();
        fake.refuse_shaders = 1;
        let error = channel.disable(1, &mut render, &mut fake).unwrap_err();
        assert_eq!(error.code, "backend");
        // mpv kept the old list, and so does the channel.
        assert_eq!(fake.shaders, ["/a", "/b"]);
        assert_eq!(channel.paths(), fake.shaders);
        assert!(!channel.holds(1), "the stopped owner holds nothing");
        // The stopped owner's shaders count for nothing and hear nothing.
        assert!(
            channel
                .compile_failure(&mut render, "ERROR: x")
                .iter()
                .all(|(owner, _, _)| *owner == 2)
        );
        // The next write leaves them out.
        set(
            &mut channel,
            &mut render,
            &mut fake,
            2,
            ("b", [1, 2]),
            "x",
            &["/b2"],
        )
        .unwrap();
        assert_eq!(fake.shaders, ["/b2"]);
        assert_eq!(channel.paths(), fake.shaders);
        // Or a later disable retries on its own.
        set(
            &mut channel,
            &mut render,
            &mut fake,
            3,
            ("c", [1, 3]),
            "x",
            &["/c"],
        )
        .unwrap();
        fake.refuse_shaders = 1;
        assert!(channel.disable(3, &mut render, &mut fake).is_err());
        assert_eq!(fake.shaders, ["/b2", "/c"]);
        channel.disable(9, &mut render, &mut fake).unwrap();
        assert_eq!(fake.shaders, ["/b2"]);
        assert_eq!(channel.paths(), fake.shaders);
    }

    /// Review M1: the message form comes from everyone whose shaders are on,
    /// not from who has not been told yet.
    #[test]
    fn a_good_list_joining_a_broken_one_is_not_blamed_with_certainty() {
        let (mut channel, mut render, mut fake) =
            (Channel::default(), Options::default(), Fake::default());
        set(
            &mut channel,
            &mut render,
            &mut fake,
            1,
            ("a", [1, 1]),
            "x",
            &["/broken"],
        )
        .unwrap();
        let first = channel.compile_failure(&mut render, "ERROR: 0:1: x");
        assert_eq!(first.len(), 1);
        assert_eq!(
            first[0].2["error"]["message"],
            "shader compile failed: ERROR: 0:1: x"
        );
        // B joins with a good shader; mpv recompiles and fails again on A's.
        set(
            &mut channel,
            &mut render,
            &mut fake,
            2,
            ("b", [1, 2]),
            "x",
            &["/good"],
        )
        .unwrap();
        let second = channel.compile_failure(&mut render, "ERROR: 0:1: x");
        assert_eq!(second.len(), 1, "only B was not told yet");
        assert_eq!(second[0].0, 2);
        let message = second[0].2["error"]["message"].as_str().unwrap();
        assert!(message.contains("cannot tell whose failed"), "{message}");
        assert!(message.contains("shaders of 2 handles"), "{message}");
    }

    /// Review M1: a broken `render().shaders` entry: a channel owner that
    /// joins is told it is ambiguous.
    #[test]
    fn a_broken_render_shader_does_not_blame_a_channel_owner_with_certainty() {
        let (mut channel, mut render, mut fake) =
            (Channel::default(), Options::default(), Fake::default());
        let file = tempfile::Builder::new().suffix(".glsl").tempfile().unwrap();
        let path = file.path().to_str().unwrap().to_owned();
        render
            .set(9, &json!({"shaders":[path]}), true, 60., &mut fake)
            .unwrap();
        let first = channel.compile_failure(&mut render, "ERROR: r");
        assert_eq!(first.len(), 1);
        assert_eq!((first[0].0, first[0].1), (9, "video.render.status"));
        assert_eq!(
            first[0].2["error"]["message"],
            "shader compile failed: ERROR: r"
        );
        set(
            &mut channel,
            &mut render,
            &mut fake,
            1,
            ("a", [1, 1]),
            "x",
            &["/good"],
        )
        .unwrap();
        let second = channel.compile_failure(&mut render, "ERROR: r");
        assert_eq!(second.len(), 1, "the render owner was told already");
        assert_eq!((second[0].0, second[0].1), (1, "video.shaders.status"));
        assert!(
            second[0].2["error"]["message"]
                .as_str()
                .unwrap()
                .contains("cannot tell whose failed")
        );
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
