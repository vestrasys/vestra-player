//! `processing.video`: the video filter chain, the video view, the render
//! options and the shader channel. Also serves the requests the two filter
//! chains share, which ask for `processing.video` or `processing.audio` by
//! stage.
//!
//! Files a plugin names (a `lut3d` node's `.cube`, the shader channel's
//! shaders, player 0.5.20) must be its own: they are resolved here, inside
//! its package or data directory (`playback/plugin_files.rs`), and the
//! playback worker gets the canonical paths. The shader channel has no
//! capability of its own: like `video.render().shaders` (this capability
//! since 1.0) it only changes how the picture is drawn while the plugin
//! runs, from files the plugin ships or made, and it is revoked when the
//! plugin stops.
use super::{
    Capability, Handled, Info, Method, Needs, Recorded, Residency, forward, forward_media,
};
use crate::host::{Host, Reply, RpcError};
use crate::playback::{plugin_files, video_shaders};
use serde_json::{Value, json};

pub struct ProcessingVideo;
static INFO: Info = Info {
    name: "processing.video",
    label: "Process the picture (filters, shaders)",
    risky: false,
    requires: &["playback.read"],
    residency: Residency::Live,
};
const BY_STAGE: Needs = Needs::Param {
    key: "stage",
    cases: &[("video", Some("processing.video"))],
    otherwise: Some("processing.audio"),
};
static METHODS: &[Method] = &[
    Method {
        name: "processing.replace",
        needs: BY_STAGE,
    },
    Method {
        name: "processing.operation",
        needs: BY_STAGE,
    },
    Method::this("video.view.set"),
    Method::this("video.view.release"),
    Method::this("video.render.set"),
    Method::this("video.render.release"),
    Method::this("video.shaders.set"),
];
impl Capability for ProcessingVideo {
    fn info(&self) -> &'static Info {
        &INFO
    }
    fn methods(&self) -> &'static [Method] {
        METHODS
    }
    fn record(&self, _: &Host, _: u64, _: &Value) -> Recorded {
        Recorded::Nothing
    }
    fn rekey(&self, _: &mut Host, _: u64, _: u64) {}
    fn handle(&self, host: &mut Host, reply: &Reply, method: &str, params: &Value) -> Handled {
        match method {
            "processing.replace" => {
                let params = lut_files(host, reply.owner, params)?;
                forward(host, reply, method, &params)
            }
            "processing.operation" => forward(host, reply, method, params),
            "video.shaders.set" => {
                let params = shader_request(host, reply.owner, params)?;
                forward_media(host, reply, method, &params)
            }
            _ => forward_media(host, reply, method, params),
        }
    }
}

/// A video change with its `lut3d` files resolved to their canonical paths
/// (`plugin_files::lut`); anything else as sent (the worker validates it).
fn lut_files(host: &Host, owner: u64, params: &Value) -> Result<Value, RpcError> {
    let has_lut = |params: &Value| {
        params["stage"] == "video"
            && params["nodes"]
                .as_array()
                .is_some_and(|nodes| nodes.iter().any(|n| n["kind"] == "lut3d"))
    };
    if !has_lut(params) {
        return Ok(params.clone());
    }
    let roots = host.image_roots(owner);
    let mut params = params.clone();
    if let Some(nodes) = params["nodes"].as_array_mut() {
        for node in nodes.iter_mut().filter(|n| n["kind"] == "lut3d") {
            if let Some(file) = node["file"].as_str() {
                node["file"] = plugin_files::lut(file, &roots)?.into();
            }
        }
    }
    Ok(params)
}

/// `video.shaders.set { id, paths }` as the worker gets it: the files
/// resolved (`plugin_files::shader`), with the plugin's manifest id and its
/// place in the joined list (`install_rank`), which the plugin cannot set.
fn shader_request(host: &Host, owner: u64, params: &Value) -> Result<Value, RpcError> {
    let id = params["id"]
        .as_str()
        .filter(|id| {
            !id.is_empty() && id.len() <= video_shaders::MAX_ID && !id.contains(char::is_control)
        })
        .ok_or_else(|| {
            RpcError::invalid(format!(
                "id must be 1 to {} bytes without control characters",
                video_shaders::MAX_ID
            ))
        })?;
    let list = params["paths"]
        .as_array()
        .ok_or_else(|| RpcError::invalid("paths must be an array of paths"))?;
    if list.len() > video_shaders::MAX_SHADERS {
        return Err(RpcError::new(
            "limit",
            format!(
                "at most {} shaders across all plugins",
                video_shaders::MAX_SHADERS
            ),
        ));
    }
    let roots = host.image_roots(owner);
    let paths = list
        .iter()
        .map(|path| {
            let path = path
                .as_str()
                .ok_or_else(|| RpcError::invalid("paths must be an array of paths"))?;
            plugin_files::shader(path, &roots)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let manifest = host
        .plugins
        .get(&owner)
        .map(|plugin| plugin.manifest.id.clone())
        .ok_or_else(|| RpcError::new("disabled", "plugin no longer running"))?;
    let (tier, installed_at) = host.install_rank(owner);
    Ok(json!({"id":id,"paths":paths,"manifestId":manifest,"rank":[tier, installed_at]}))
}
