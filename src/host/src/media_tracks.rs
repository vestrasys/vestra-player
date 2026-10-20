//! Experimental source-bound media facts, not a final SDK API. Caller must
//! capture these facts with the same source lease/generation used by the worker.
//! mpv 0.41 track-list/ff-index is only guaranteed here with demuxer=lavf;
//! track IDs are never treated as FFmpeg stream indexes.
#![forbid(unsafe_code)]

use std::collections::BTreeSet;

use anyhow::{Result, bail, ensure};
use rsmpv::{Mpv, Node};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioTrack {
    pub track_id: i64,
    pub stream_index: Option<u32>,
    pub selected: bool,
    pub external: bool,
    pub codec: Option<String>,
    pub title: Option<String>,
    pub language: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Choice {
    #[default]
    Selected,
    None,
    Track {
        track_id: i64,
    },
}

fn field<'a>(fields: &'a [(String, Node)], name: &str) -> Option<&'a Node> {
    fields
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

fn text(fields: &[(String, Node)], name: &str) -> Result<Option<String>> {
    match field(fields, name) {
        None | Some(Node::None) => Ok(None),
        Some(Node::String(value)) => {
            ensure!(
                value.len() <= 1024 && !value.contains('\0'),
                "audio {name} exceeds string limits"
            );
            Ok(Some(value.clone()))
        }
        _ => bail!("invalid audio {name} property type"),
    }
}

fn flag(fields: &[(String, Node)], name: &str) -> Result<bool> {
    match field(fields, name) {
        None | Some(Node::None) => Ok(false),
        Some(Node::Flag(value)) => Ok(*value),
        _ => bail!("invalid track {name} flag"),
    }
}

fn valid_id(id: i64) -> bool {
    (1..=(1_i64 << 53) - 1).contains(&id)
}

fn parse(node: Node) -> Result<Vec<AudioTrack>> {
    let Node::Array(tracks) = node else {
        bail!("track-list is not an array");
    };
    ensure!(tracks.len() <= 1024, "track-list exceeds 1024 tracks");
    let mut result = Vec::new();
    let mut ids = BTreeSet::new();
    for track in tracks {
        let Node::Map(fields) = track else {
            bail!("track-list entry is not a map");
        };
        let Some(Node::String(kind)) = field(&fields, "type") else {
            bail!("track-list entry has no valid type");
        };
        if kind != "audio" {
            continue;
        }
        let Some(Node::Int64(id)) = field(&fields, "id") else {
            bail!("audio track has no integer id");
        };
        ensure!(
            valid_id(*id) && ids.insert(*id),
            "audio track IDs must be unique positive safe integers"
        );
        // Missing, malformed or out-of-range ff-index means unknown, not an
        // inferred index. Non-lavf demuxers are rejected separately at resolve.
        let stream_index = match field(&fields, "ff-index") {
            Some(Node::Int64(index)) if (0..=1023).contains(index) => Some(*index as u32),
            _ => None,
        };
        result.push(AudioTrack {
            track_id: *id,
            stream_index,
            selected: flag(&fields, "selected")?,
            external: flag(&fields, "external")?,
            codec: text(&fields, "codec")?,
            title: text(&fields, "title")?,
            language: text(&fields, "lang")?,
        });
    }
    Ok(result)
}

/// Read only public native track-list facts. This does not select a track or
/// prove the eventual offline decoder supports its codec.
pub fn audio_tracks(mpv: &Mpv) -> Result<Vec<AudioTrack>> {
    parse(mpv.get_property::<Node>("track-list")?)
}

/// A selected video track that is not cover art: frames to step through.
pub fn moving_video(node: &Node) -> bool {
    let Node::Array(tracks) = node else {
        return false;
    };
    tracks.iter().any(|track| {
        let Node::Map(fields) = track else {
            return false;
        };
        matches!(field(fields, "type"), Some(Node::String(kind)) if kind == "video")
            && matches!(field(fields, "selected"), Some(Node::Flag(true)))
            && !matches!(field(fields, "albumart"), Some(Node::Flag(true)))
    })
}

fn selected_video_stream(node: Node, demuxer: &str) -> Result<u32> {
    ensure!(
        demuxer == "lavf",
        "offline video mapping requires the lavf demuxer"
    );
    let Node::Array(tracks) = node else {
        bail!("track-list is not an array");
    };
    ensure!(tracks.len() <= 1024, "track-list exceeds 1024 tracks");
    let mut selected = None;
    let mut ids = BTreeSet::new();
    for track in tracks {
        let Node::Map(fields) = track else {
            bail!("track-list entry is not a map");
        };
        let Some(Node::String(kind)) = field(&fields, "type") else {
            bail!("track-list entry has no valid type");
        };
        if kind != "video" {
            continue;
        }
        let Some(Node::Int64(id)) = field(&fields, "id") else {
            bail!("video track has no integer id");
        };
        ensure!(
            valid_id(*id) && ids.insert(*id),
            "video track IDs must be unique positive safe integers"
        );
        if !flag(&fields, "selected")? {
            continue;
        }
        ensure!(
            selected.is_none(),
            "multiple selected video tracks are unsupported"
        );
        ensure!(
            !flag(&fields, "external")?,
            "external video cannot use the source lease"
        );
        ensure!(
            !flag(&fields, "albumart")? && !flag(&fields, "image")?,
            "cover art and still-image tracks are not supported media video"
        );
        let index = match field(&fields, "ff-index") {
            Some(Node::Int64(index)) if (0..=1023).contains(index) => *index as u32,
            _ => bail!("selected video FFmpeg stream index is unavailable"),
        };
        selected = Some(index);
    }
    selected.ok_or_else(|| anyhow::anyhow!("no selected media video track"))
}

/// Resolve the actual selected video from the public track list, never v:0 or
/// track ID arithmetic. Caller holds the actual-FileLoaded/source gate and must
/// freeze the result alongside that source lease before dispatching a worker.
pub fn video_stream(mpv: &Mpv) -> Result<u32> {
    let demuxer = mpv.get_property::<String>("current-demuxer")?;
    selected_video_stream(mpv.get_property::<Node>("track-list")?, &demuxer)
}

pub fn resolve(
    tracks: &[AudioTrack],
    demuxer: &str,
    choice: &Choice,
) -> Result<Option<AudioTrack>> {
    if matches!(choice, Choice::None) {
        return Ok(None);
    }
    ensure!(tracks.len() <= 1024, "audio facts exceed 1024 tracks");
    let mut ids = BTreeSet::new();
    for track in tracks {
        ensure!(
            valid_id(track.track_id) && ids.insert(track.track_id),
            "audio track IDs must be unique positive safe integers"
        );
        for value in [&track.codec, &track.title, &track.language]
            .into_iter()
            .flatten()
        {
            ensure!(
                value.len() <= 1024 && !value.contains('\0'),
                "audio metadata exceeds string limits"
            );
        }
    }
    let track = match choice {
        Choice::None => unreachable!(),
        Choice::Selected => {
            let mut selected = tracks.iter().filter(|track| track.selected);
            let first = selected.next();
            ensure!(
                selected.next().is_none(),
                "multiple selected audio tracks are unsupported"
            );
            let Some(track) = first else {
                return Ok(None);
            };
            track
        }
        Choice::Track { track_id } => {
            ensure!(valid_id(*track_id), "requested audio track ID is invalid");
            tracks
                .iter()
                .find(|track| track.track_id == *track_id)
                .ok_or_else(|| anyhow::anyhow!("requested audio track does not exist"))?
        }
    };
    ensure!(
        !track.external,
        "external audio tracks cannot use the source lease"
    );
    ensure!(
        demuxer == "lavf",
        "offline audio mapping requires the lavf demuxer"
    );
    ensure!(
        track.stream_index.is_some_and(|index| index <= 1023),
        "audio FFmpeg stream index is unavailable"
    );
    Ok(Some(track.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video_node(id: i64, index: i64, selected: bool) -> Node {
        Node::Map(vec![
            ("type".into(), Node::String("video".into())),
            ("id".into(), Node::Int64(id)),
            ("ff-index".into(), Node::Int64(index)),
            ("selected".into(), Node::Flag(selected)),
        ])
    }

    #[test]
    fn actual_second_selected_video_uses_its_absolute_lavf_index() {
        let tracks = Node::Array(vec![video_node(1, 0, false), video_node(8, 5, true)]);
        assert_eq!(selected_video_stream(tracks.clone(), "lavf").unwrap(), 5);
        assert!(selected_video_stream(tracks, "mkv").is_err());
        assert!(selected_video_stream(Node::Array(vec![video_node(1, 0, false)]), "lavf").is_err());
    }

    #[test]
    fn selected_video_refuses_covers_external_missing_index_and_ambiguity() {
        for key in ["external", "albumart", "image"] {
            let Node::Map(mut fields) = video_node(1, 0, true) else {
                unreachable!()
            };
            fields.push((key.into(), Node::Flag(true)));
            assert!(selected_video_stream(Node::Array(vec![Node::Map(fields)]), "lavf").is_err());
        }
        let Node::Map(mut fields) = video_node(1, 0, true) else {
            unreachable!()
        };
        fields.retain(|(key, _)| key != "ff-index");
        assert!(selected_video_stream(Node::Array(vec![Node::Map(fields)]), "lavf").is_err());
        for index in [-1, 1024] {
            assert!(
                selected_video_stream(Node::Array(vec![video_node(1, index, true)]), "lavf")
                    .is_err()
            );
        }
        assert!(
            selected_video_stream(
                Node::Array(vec![video_node(1, 0, true), video_node(2, 3, true)]),
                "lavf"
            )
            .is_err()
        );
        assert!(selected_video_stream(Node::Array(vec![video_node(0, 0, true)]), "lavf").is_err());
        assert!(selected_video_stream(Node::Array(vec![Node::None; 1025]), "lavf").is_err());
    }

    fn track(id: i64, selected: bool, index: u32) -> AudioTrack {
        AudioTrack {
            track_id: id,
            stream_index: Some(index),
            selected,
            external: false,
            codec: Some("aac".into()),
            title: None,
            language: None,
        }
    }

    #[test]
    fn selected_none_and_explicit_second_track_use_real_stream_index() {
        let tracks = [track(1, true, 2), track(7, false, 4)];
        assert_eq!(
            resolve(&tracks, "lavf", &Choice::Selected)
                .unwrap()
                .unwrap()
                .stream_index,
            Some(2)
        );
        let second = resolve(&tracks, "lavf", &Choice::Track { track_id: 7 })
            .unwrap()
            .unwrap();
        assert_eq!((second.track_id, second.stream_index), (7, Some(4)));
        assert!(resolve(&tracks, "other", &Choice::None).unwrap().is_none());
        assert!(resolve(&[], "other", &Choice::Selected).unwrap().is_none());
    }

    #[test]
    fn refuses_ambiguous_or_unleased_audio() {
        assert!(
            resolve(
                &[track(1, true, 0), track(2, true, 1)],
                "lavf",
                &Choice::Selected
            )
            .is_err()
        );
        assert!(
            resolve(
                &[track(1, true, 0), track(1, false, 1)],
                "lavf",
                &Choice::Selected
            )
            .is_err()
        );
        assert!(resolve(&[track(1, true, 0)], "mkv", &Choice::Selected).is_err());
        assert!(resolve(&[track(1, true, 0)], "lavf", &Choice::Track { track_id: 2 }).is_err());
        let mut external = track(1, true, 0);
        external.external = true;
        assert!(resolve(&[external], "lavf", &Choice::Selected).is_err());
        let mut unknown = track(1, true, 0);
        unknown.stream_index = None;
        assert!(resolve(&[unknown], "lavf", &Choice::Selected).is_err());
    }

    #[test]
    fn native_facts_bound_metadata_and_never_guess_index() {
        let native = |index| {
            Node::Array(vec![Node::Map(vec![
                ("type".into(), Node::String("audio".into())),
                ("id".into(), Node::Int64(9)),
                ("ff-index".into(), Node::Int64(index)),
            ])])
        };
        for index in [-1, 1024] {
            assert!(parse(native(index)).unwrap()[0].stream_index.is_none());
        }
        assert_eq!(parse(native(0)).unwrap()[0].stream_index, Some(0));
        let Node::Array(mut rows) = native(0) else {
            unreachable!()
        };
        let Node::Map(fields) = &mut rows[0] else {
            unreachable!()
        };
        fields.push(("title".into(), Node::String("bad\0title".into())));
        assert!(parse(Node::Array(rows)).is_err());
        assert!(parse(Node::Array(vec![Node::None; 1025])).is_err());
    }
}
