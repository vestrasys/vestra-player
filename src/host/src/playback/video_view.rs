//! The display layer `plugin.video.view()` (R-V9): aspect override, fit mode,
//! zoom and pan as mpv vo properties. One owner at a time (like the sequence
//! provider); the owner's release or disable writes the defaults back.
//! Display settings never enter snapshots or exports (those run offline FFmpeg).
#![forbid(unsafe_code)]

use crate::host::RpcError;
use rsmpv::Mpv;
use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fit {
    Keep,
    Stretch,
    Fill,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// `None`: as the source.
    pub aspect: Option<f64>,
    pub fit: Fit,
    pub zoom: f64,
    pub pan_x: f64,
    pub pan_y: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            aspect: None,
            fit: Fit::Keep,
            zoom: 1.,
            pan_x: 0.,
            pan_y: 0.,
        }
    }
}
impl Settings {
    /// `values` merged onto `self`; unknown fields and out-of-range values are
    /// `invalid` and change nothing.
    pub fn merged(self, values: &Value) -> Result<Self, RpcError> {
        let values = values
            .as_object()
            .ok_or_else(|| RpcError::invalid("values must be an object"))?;
        let number = |key: &str, min: f64, max: f64| -> Result<f64, RpcError> {
            values[key]
                .as_f64()
                .filter(|v| v.is_finite() && (min..=max).contains(v))
                .ok_or_else(|| RpcError::invalid(format!("{key} must be a number in {min}..{max}")))
        };
        let mut next = self;
        for (key, value) in values {
            match key.as_str() {
                "aspect" if value.is_null() => next.aspect = None,
                "aspect" => next.aspect = Some(number("aspect", 0.25, 4.)?),
                "fit" => {
                    next.fit = match value.as_str() {
                        Some("keep") => Fit::Keep,
                        Some("stretch") => Fit::Stretch,
                        Some("fill") => Fit::Fill,
                        _ => return Err(RpcError::invalid("fit must be keep, stretch or fill")),
                    }
                }
                "zoom" => next.zoom = number("zoom", 0.25, 4.)?,
                "panX" => next.pan_x = number("panX", -1., 1.)?,
                "panY" => next.pan_y = number("panY", -1., 1.)?,
                other => return Err(RpcError::invalid(format!("unknown view field {other}"))),
            }
        }
        Ok(next)
    }
    /// The mpv properties, as option strings.
    pub fn properties(self) -> [(&'static str, String); 6] {
        let (keepaspect, panscan) = match self.fit {
            Fit::Keep => ("yes", "0"),
            Fit::Stretch => ("no", "0"),
            Fit::Fill => ("yes", "1"),
        };
        [
            (
                "video-aspect-override",
                self.aspect.map_or("-1".into(), |a| a.to_string()),
            ),
            ("keepaspect", keepaspect.into()),
            ("panscan", panscan.into()),
            ("video-zoom", self.zoom.log2().to_string()),
            ("video-pan-x", self.pan_x.to_string()),
            ("video-pan-y", self.pan_y.to_string()),
        ]
    }
}

/// Who owns the display layer and what it last asked for.
#[derive(Debug, Default)]
pub struct View {
    owner: Option<u64>,
    settings: Settings,
}
impl View {
    #[cfg(test)]
    pub fn owner(&self) -> Option<u64> {
        self.owner
    }
    pub fn settings(&self) -> Settings {
        self.settings
    }
    /// `video.view.set { values }` → `{ state }`: takes the layer when free
    /// (`conflict` while another plugin holds it), merges, writes mpv.
    /// mpv keeps these options across loads, so without ready media they are
    /// written anyway and reported as `waiting`.
    pub fn set(
        &mut self,
        owner: u64,
        values: &Value,
        ready: bool,
        write: impl FnMut(&str, &str) -> Result<(), String>,
    ) -> Result<Value, RpcError> {
        if self.owner.is_some_and(|held| held != owner) {
            return Err(RpcError::new(
                "conflict",
                "another plugin owns the display layer",
            ));
        }
        let next = self.settings.merged(values)?;
        self.owner = Some(owner);
        self.write(next, write)?;
        Ok(json!({"state": if ready { "applied" } else { "waiting" }}))
    }
    /// `video.view.release`, or the owner was disabled: defaults, layer free.
    /// Other plugins' calls change nothing.
    pub fn release(
        &mut self,
        owner: u64,
        write: impl FnMut(&str, &str) -> Result<(), String>,
    ) -> Result<bool, RpcError> {
        if self.owner != Some(owner) {
            return Ok(false);
        }
        self.owner = None;
        self.write(Settings::default(), write)?;
        Ok(true)
    }
    /// Writes every property; on a refusal the previous values are restored
    /// (best effort) and the call fails with `backend`.
    fn write(
        &mut self,
        next: Settings,
        mut write: impl FnMut(&str, &str) -> Result<(), String>,
    ) -> Result<(), RpcError> {
        let failed = next
            .properties()
            .iter()
            .find_map(|(name, value)| write(name, value).err().map(|e| format!("{name}: {e}")));
        if let Some(error) = failed {
            for (name, value) in self.settings.properties() {
                let _ = write(name, &value);
            }
            return Err(RpcError::new("backend", error));
        }
        self.settings = next;
        Ok(())
    }
}

/// Writes one property on the real player.
pub fn mpv_writer(mpv: &Mpv) -> impl FnMut(&str, &str) -> Result<(), String> + '_ {
    move |name, value| mpv.set_property(name, value).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    type Props = BTreeMap<String, String>;
    fn record(props: &mut Props, name: &str, value: &str) -> Result<(), String> {
        props.insert(name.into(), value.into());
        Ok(())
    }

    #[test]
    fn fields_map_to_mpv_properties() {
        let view = Settings::default().properties();
        assert_eq!(
            view.map(|(n, v)| format!("{n}={v}")),
            [
                "video-aspect-override=-1",
                "keepaspect=yes",
                "panscan=0",
                "video-zoom=0",
                "video-pan-x=0",
                "video-pan-y=0"
            ]
        );
        let settings = Settings::default()
            .merged(&json!({"aspect":1.3333,"fit":"fill","zoom":2,"panX":-0.5,"panY":1}))
            .unwrap();
        let map: BTreeMap<_, _> = settings.properties().into_iter().collect();
        assert_eq!(map["video-aspect-override"], "1.3333");
        assert_eq!(
            (map["keepaspect"].as_str(), map["panscan"].as_str()),
            ("yes", "1")
        );
        assert_eq!(map["video-zoom"], "1", "log2(2)");
        assert_eq!(map["video-pan-x"], "-0.5");
        let stretch = settings
            .merged(&json!({"fit":"stretch","zoom":0.25}))
            .unwrap();
        let map: BTreeMap<_, _> = stretch.properties().into_iter().collect();
        assert_eq!(
            (map["keepaspect"].as_str(), map["panscan"].as_str()),
            ("no", "0")
        );
        assert_eq!(map["video-zoom"], "-2");
        assert_eq!(stretch.aspect, Some(1.3333), "merged, not replaced");
        assert_eq!(
            stretch.merged(&json!({"aspect":null})).unwrap().aspect,
            None
        );
    }

    #[test]
    fn out_of_range_or_unknown_values_are_invalid_and_change_nothing() {
        for values in [
            json!({"aspect":0.2}),
            json!({"aspect":5}),
            json!({"zoom":0}),
            json!({"zoom":4.5}),
            json!({"panX":-1.1}),
            json!({"panY":"0"}),
            json!({"fit":"crop"}),
            json!({"scaleX":1}),
            json!([1]),
        ] {
            let error = Settings::default().merged(&values).unwrap_err();
            assert_eq!(error.code, "invalid", "{values}");
        }
        let mut props = Props::new();
        let mut view = View::default();
        view.set(1, &json!({"zoom":2}), true, |n, v| record(&mut props, n, v))
            .unwrap();
        let before = props.clone();
        let error = view
            .set(1, &json!({"zoom":2,"panX":3}), true, |n, v| {
                record(&mut props, n, v)
            })
            .unwrap_err();
        assert_eq!(error.code, "invalid");
        assert_eq!(props, before);
        assert_eq!(view.settings().zoom, 2.);
    }

    #[test]
    fn one_owner_until_it_releases_and_defaults_come_back() {
        let mut props = Props::new();
        let mut view = View::default();
        let state = view
            .set(1, &json!({}), false, |n, v| record(&mut props, n, v))
            .unwrap();
        assert_eq!(state["state"], "waiting", "no ready media");
        assert_eq!(view.owner(), Some(1), "the first call takes the layer");
        let state = view
            .set(1, &json!({"aspect":1.5,"zoom":4}), true, |n, v| {
                record(&mut props, n, v)
            })
            .unwrap();
        assert_eq!(state["state"], "applied");
        let error = view
            .set(2, &json!({"aspect":1}), true, |n, v| {
                record(&mut props, n, v)
            })
            .unwrap_err();
        assert_eq!(error.code, "conflict");
        assert_eq!(props["video-aspect-override"], "1.5");
        // Another plugin's release does nothing.
        assert!(!view.release(2, |n, v| record(&mut props, n, v)).unwrap());
        assert!(view.release(1, |n, v| record(&mut props, n, v)).unwrap());
        assert_eq!(view.owner(), None);
        assert_eq!(view.settings(), Settings::default());
        assert_eq!(props["video-aspect-override"], "-1");
        assert_eq!(props["video-zoom"], "0");
        view.set(2, &json!({"fit":"stretch"}), true, |n, v| {
            record(&mut props, n, v)
        })
        .unwrap();
        assert_eq!(view.owner(), Some(2), "free again");
    }

    #[test]
    fn a_refused_property_restores_the_previous_values() {
        let mut props = BTreeMap::<String, String>::new();
        let mut view = View::default();
        let refuse = std::cell::Cell::new(false);
        let write = |props: &mut BTreeMap<String, String>, name: &str, value: &str| {
            if refuse.get() && name == "video-pan-x" && value != "0" {
                return Err("property unavailable".to_string());
            }
            props.insert(name.into(), value.into());
            Ok(())
        };
        view.set(1, &json!({"zoom":2}), true, |n, v| write(&mut props, n, v))
            .unwrap();
        refuse.set(true);
        let error = view
            .set(1, &json!({"zoom":4,"panX":0.5}), true, |n, v| {
                write(&mut props, n, v)
            })
            .unwrap_err();
        assert_eq!(error.code, "backend");
        assert_eq!(props["video-zoom"], "1", "back to zoom 2");
        assert_eq!(view.settings().zoom, 2.);
    }
}
