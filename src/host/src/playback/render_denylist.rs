//! Graphics drivers the player does not render on with the GPU: the rules
//! built into the host (`src/host/render-denylist.json`) and a local file of
//! the same shape that adds or replaces rules by id, so a rule can change
//! without a player release (doc/architecture/README.md §4.3). The
//! format follows Chromium's `software_rendering_list.json`. Pure: the
//! caller reads the local file and the adapter facts.
#![forbid(unsafe_code)]

use crate::platform::GpuAdapter;
use serde::Deserialize;
use std::cmp::Ordering;

const BUILT_IN: &str = include_str!("../../render-denylist.json");

/// What a matching rule does.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Software rendering.
    Cpu,
    /// The player does not draw its windows with the rule's `window_api`
    /// (the driver cannot draw them): the next window renderer takes over
    /// (Direct3D 12 → OpenGL → Slint's software renderer), and where GPU
    /// rendering needs those windows it renders in software.
    SoftwareWindows,
    /// A `render:` note only.
    Warn,
}

/// The graphics API of the windows a `software-windows` rule is about.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WindowApi {
    /// femtovg on the driver's OpenGL (the default, as before 0.5.19).
    /// Matched against every adapter with a GPU of its own: OpenGL windows
    /// may draw on another GPU than the one GPU rendering uses.
    #[default]
    Opengl,
    /// femtovg on wgpu, Direct3D 12 (Windows). Matched against the adapter
    /// GPU rendering uses, which the windows' device is opened on.
    Direct3d12,
}

/// A version condition: `op` is `<`, `<=`, `=`, `>=`, `>` or `between`
/// (`value` to `value2`, both included).
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Version {
    pub op: String,
    pub value: String,
    #[serde(default)]
    pub value2: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Os {
    /// `windows`, `macos` or `linux` (`std::env::consts::OS`).
    #[serde(rename = "type")]
    pub kind: String,
    /// The system build. Dotted numbers (Windows `26200.6899`) compare part
    /// by part with every `op`; a build with letters (macOS `25C56`) only
    /// with `=`, as text.
    #[serde(default)]
    pub build: Option<Version>,
}

/// The conditions of a rule or of one of its exceptions; all given ones
/// must hold. A condition on the adapter never holds without one.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Conditions {
    #[serde(default)]
    pub os: Option<Os>,
    /// `0x1002`.
    #[serde(default)]
    pub vendor_id: Option<String>,
    /// Any of these (`0x15BF`).
    #[serde(default)]
    pub device_id: Vec<String>,
    #[serde(default)]
    pub driver_version: Option<Version>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub refs: Vec<String>,
    /// A local rule that switches a built-in one off.
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub action: Option<Action>,
    /// With `software-windows`: which windows (`opengl`, the default, or
    /// `direct3d12`). Refused with another action.
    #[serde(default)]
    pub window_api: Option<WindowApi>,
    #[serde(flatten)]
    pub conditions: Conditions,
    #[serde(default)]
    pub exceptions: Vec<Conditions>,
}

#[derive(Deserialize)]
struct File {
    version: u32,
    rules: Vec<Rule>,
}

/// What the rules are matched against.
#[derive(Clone, Copy, Debug)]
pub struct Facts<'a> {
    /// `std::env::consts::OS`.
    pub os: &'a str,
    pub build: Option<&'a str>,
    /// The adapter GPU rendering would use, if known.
    pub adapter: Option<&'a GpuAdapter>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Denylist {
    rules: Vec<Rule>,
}

impl Denylist {
    /// The rules built into the host (checked by the tests, so never empty
    /// by accident).
    pub fn built_in() -> Self {
        Self::default()
            .merged(BUILT_IN)
            .unwrap_or_else(|error| panic!("render-denylist.json: {error}"))
    }
    /// These rules with `text`'s (a denylist file) added; a rule with an id
    /// already present replaces it. A rule without an action that does not
    /// switch one off is refused, as is a file of another version.
    pub fn merged(mut self, text: &str) -> Result<Self, String> {
        let file: File = serde_json::from_str(text).map_err(|error| error.to_string())?;
        if file.version != 1 {
            return Err(format!("version {} is not 1", file.version));
        }
        for rule in file.rules {
            if !rule.disabled && rule.action.is_none() {
                return Err(format!("rule {} has no action", rule.id));
            }
            if rule.window_api.is_some() && rule.action != Some(Action::SoftwareWindows) {
                return Err(format!(
                    "rule {}: window_api needs the action software-windows",
                    rule.id
                ));
            }
            for conditions in std::iter::once(&rule.conditions).chain(&rule.exceptions) {
                conditions
                    .check()
                    .map_err(|error| format!("rule {}: {error}", rule.id))?;
            }
            self.rules.retain(|old| old.id != rule.id);
            self.rules.push(rule);
        }
        Ok(self)
    }
    /// The rules that hold for `facts`, in file order.
    pub fn matches(&self, facts: Facts<'_>) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|rule| {
                !rule.disabled
                    && rule.conditions.hold(facts)
                    && !rule
                        .exceptions
                        .iter()
                        .any(|exception| exception.hold(facts))
            })
            .collect()
    }
}

impl Rule {
    /// The windows a `software-windows` rule is about (`None` for another
    /// action).
    pub fn windows(&self) -> Option<WindowApi> {
        (self.action == Some(Action::SoftwareWindows)).then(|| self.window_api.unwrap_or_default())
    }
}

impl Conditions {
    /// Ids are hexadecimal; versions have a known `op`.
    fn check(&self) -> Result<(), String> {
        for id in self.vendor_id.iter().chain(&self.device_id) {
            if hex(id).is_none() {
                return Err(format!("{id:?} is not a hexadecimal id"));
            }
        }
        let builds = self.os.as_ref().and_then(|os| os.build.as_ref());
        for version in builds.into_iter().chain(&self.driver_version) {
            version.check()?;
        }
        Ok(())
    }
    fn hold(&self, facts: Facts<'_>) -> bool {
        let os = self.os.as_ref().is_none_or(|os| {
            os.kind == facts.os
                && os
                    .build
                    .as_ref()
                    .is_none_or(|build| facts.build.is_some_and(|value| build.holds(value)))
        });
        let adapter = |test: &dyn Fn(&GpuAdapter) -> bool| facts.adapter.is_some_and(test);
        let vendor = self
            .vendor_id
            .as_deref()
            .is_none_or(|id| adapter(&|a| hex(id) == Some(a.vendor_id)));
        let device = self.device_id.is_empty()
            || adapter(&|a| self.device_id.iter().any(|id| hex(id) == Some(a.device_id)));
        let driver = self.driver_version.as_ref().is_none_or(|version| {
            adapter(&|a| {
                a.driver_version
                    .as_deref()
                    .is_some_and(|v| version.holds(v))
            })
        });
        os && vendor && device && driver
    }
}

impl Version {
    /// A known `op`, `value2` with `between`.
    fn check(&self) -> Result<(), String> {
        match self.op.as_str() {
            "<" | "<=" | "=" | "==" | ">=" | ">" => Ok(()),
            "between" if self.value2.is_some() => Ok(()),
            "between" => Err("between without value2".into()),
            other => Err(format!("unknown op {other:?}")),
        }
    }
    fn holds(&self, value: &str) -> bool {
        let against = |bound: &str| compare(value, bound);
        match self.op.as_str() {
            "<" => against(&self.value) == Some(Ordering::Less),
            "<=" => matches!(against(&self.value), Some(Ordering::Less | Ordering::Equal)),
            "=" | "==" => match against(&self.value) {
                Some(order) => order == Ordering::Equal,
                // Not dotted numbers (a macOS build): the same text.
                None => value == self.value,
            },
            ">=" => matches!(
                against(&self.value),
                Some(Ordering::Greater | Ordering::Equal)
            ),
            ">" => against(&self.value) == Some(Ordering::Greater),
            "between" => {
                matches!(
                    against(&self.value),
                    Some(Ordering::Greater | Ordering::Equal)
                ) && self.value2.as_deref().is_some_and(|high| {
                    matches!(against(high), Some(Ordering::Less | Ordering::Equal))
                })
            }
            _ => false,
        }
    }
}

/// `0x15BF` or `15bf`.
fn hex(text: &str) -> Option<u32> {
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    u32::from_str_radix(digits, 16).ok()
}

/// Dotted versions part by part as numbers, missing parts as 0; `None`
/// when a part is not a number.
pub fn compare(a: &str, b: &str) -> Option<Ordering> {
    let parts = |text: &str| {
        text.split('.')
            .map(|part| part.trim().parse::<u64>().ok())
            .collect::<Option<Vec<_>>>()
    };
    let (a, b) = (parts(a)?, parts(b)?);
    let length = a.len().max(b.len());
    let at = |list: &[u64], i: usize| list.get(i).copied().unwrap_or(0);
    Some(
        (0..length)
            .map(|i| at(&a, i).cmp(&at(&b, i)))
            .find(|order| order.is_ne())
            .unwrap_or(Ordering::Equal),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter(vendor: u32, device: u32, driver: &str) -> GpuAdapter {
        GpuAdapter {
            name: "Test GPU".into(),
            vendor_id: vendor,
            device_id: device,
            driver_version: Some(driver.into()),
            software: false,
            luid: 0,
        }
    }
    fn on_windows(adapter: &GpuAdapter) -> Facts<'_> {
        Facts {
            os: "windows",
            build: Some("26200"),
            adapter: Some(adapter),
        }
    }

    #[test]
    fn the_built_in_rules_hold_for_their_780m_drivers_only() {
        let list = Denylist::built_in();
        let ids = |a: &GpuAdapter| -> Vec<String> {
            list.matches(on_windows(a))
                .iter()
                .map(|rule| rule.id.clone())
                .collect()
        };
        let bad = adapter(0x1002, 0x15BF, "31.0.14005.21001");
        assert_eq!(ids(&bad), ["amd-780m-draw-stalls"]);
        assert_eq!(list.matches(on_windows(&bad))[0].action, Some(Action::Cpu));
        // Adrenalin 26.10.1 and later: white OpenGL windows.
        let white = adapter(0x1002, 0x15BF, "32.0.32015.7029");
        assert_eq!(ids(&white), ["amd-780m-white-opengl-windows"]);
        assert_eq!(
            list.matches(on_windows(&white))[0].action,
            Some(Action::SoftwareWindows)
        );
        assert_eq!(
            ids(&adapter(0x1002, 0x15BF, "33.0.1.1")).len(),
            1,
            "no upper bound yet"
        );
        // Between the two, another device, another vendor, another system,
        // no driver version, no adapter.
        assert!(ids(&adapter(0x1002, 0x15BF, "32.0.21000.1")).is_empty());
        assert!(ids(&adapter(0x1002, 0x1681, "31.0.14005.21001")).is_empty());
        assert!(ids(&adapter(0x10DE, 0x2820, "32.0.16.1062")).is_empty());
        let mut unknown = bad.clone();
        unknown.driver_version = None;
        assert!(ids(&unknown).is_empty());
        let mac = Facts {
            os: "macos",
            build: Some("25C56"),
            adapter: Some(&bad),
        };
        assert!(list.matches(mac).is_empty());
        let none = Facts {
            os: "windows",
            build: None,
            adapter: None,
        };
        assert!(list.matches(none).is_empty());
    }

    #[test]
    fn a_local_file_adds_replaces_and_switches_rules_off() {
        let bad = adapter(0x1002, 0x15BF, "31.0.14005.21001");
        let off = Denylist::built_in()
            .merged(r#"{"version":1,"rules":[{"id":"amd-780m-draw-stalls","disabled":true}]}"#)
            .unwrap();
        assert!(off.matches(on_windows(&bad)).is_empty());
        let added = Denylist::built_in()
            .merged(
                r#"{"version":1,"rules":[{"id":"nv","description":"x","vendor_id":"0x10de",
                    "driver_version":{"op":"<","value":"32.0.16"},"action":"warn",
                    "exceptions":[{"device_id":["0x2820"]}]}]}"#,
            )
            .unwrap();
        let old_nvidia = adapter(0x10DE, 0x2504, "31.0.15.5222");
        let rules = added.matches(on_windows(&old_nvidia));
        assert_eq!(
            (rules[0].id.as_str(), rules[0].action),
            ("nv", Some(Action::Warn))
        );
        assert!(
            added
                .matches(on_windows(&adapter(0x10DE, 0x2820, "31.0.15.1")))
                .is_empty()
        );
        assert_eq!(
            added.matches(on_windows(&bad)).len(),
            1,
            "built-in rule kept"
        );
        let replaced = Denylist::built_in()
            .merged(r#"{"version":1,"rules":[{"id":"amd-780m-draw-stalls","action":"warn","vendor_id":"0x1002"}]}"#)
            .unwrap();
        assert_eq!(
            replaced.matches(on_windows(&bad))[0].action,
            Some(Action::Warn)
        );
        for bad_file in [
            r#"{"version":1,"rules":[{"id":"x","action":"cpu","vendor_id":"AMD"}]}"#,
            r#"{"version":1,"rules":[{"id":"x","action":"cpu","device_id":["0x15BF","?"]}]}"#,
            r#"{"version":1,"rules":[{"id":"x","action":"cpu","driver_version":{"op":"~","value":"1"}}]}"#,
            r#"{"version":1,"rules":[{"id":"x","action":"cpu","driver_version":{"op":"between","value":"1"}}]}"#,
            r#"{"version":1,"rules":[{"id":"x","action":"cpu","exceptions":[{"vendor_id":"zz"}]}]}"#,
            "{",
            r#"{"version":2,"rules":[]}"#,
            r#"{"version":1,"rules":[{"id":"x"}]}"#,
        ] {
            assert!(Denylist::default().merged(bad_file).is_err(), "{bad_file}");
        }
    }

    /// A rule can name Direct3D 12 (wgpu) windows; a
    /// `software-windows` rule without `window_api` is about OpenGL ones,
    /// as before.
    #[test]
    fn a_window_rule_names_its_windows_api() {
        let list = Denylist::default()
            .merged(
                r#"{"version":1,"rules":[
                    {"id":"d3d","action":"software-windows","window_api":"direct3d12","vendor_id":"0x8086"},
                    {"id":"gl","action":"software-windows","vendor_id":"0x8086"},
                    {"id":"cpu","action":"cpu","vendor_id":"0x8086"}]}"#,
            )
            .unwrap();
        let intel = adapter(0x8086, 0x46A6, "31.0.101.5186");
        let windows: Vec<_> = list
            .matches(on_windows(&intel))
            .iter()
            .map(|rule| (rule.id.as_str(), rule.windows()))
            .collect();
        assert_eq!(
            windows,
            [
                ("d3d", Some(WindowApi::Direct3d12)),
                ("gl", Some(WindowApi::Opengl)),
                ("cpu", None)
            ]
        );
        for bad in [
            r#"{"version":1,"rules":[{"id":"x","action":"cpu","window_api":"direct3d12"}]}"#,
            r#"{"version":1,"rules":[{"id":"x","action":"software-windows","window_api":"vulkan"}]}"#,
        ] {
            assert!(Denylist::default().merged(bad).is_err(), "{bad}");
        }
        // The built-in white-window rule is about OpenGL windows.
        let white = adapter(0x1002, 0x15BF, "32.0.32015.7029");
        let built_in = Denylist::built_in();
        assert_eq!(
            built_in.matches(on_windows(&white))[0].windows(),
            Some(WindowApi::Opengl)
        );
    }

    #[test]
    fn versions_compare_part_by_part_and_builds_match() {
        assert_eq!(
            compare("31.0.14005.21001", "31.0.14005"),
            Some(Ordering::Greater)
        );
        assert_eq!(compare("31.0.9", "31.0.10"), Some(Ordering::Less));
        assert_eq!(compare("1.0", "1"), Some(Ordering::Equal));
        assert_eq!(compare("25C56", "1"), None);
        let version = |op: &str, value: &str| Version {
            op: op.into(),
            value: value.into(),
            value2: None,
        };
        assert!(version("<", "32.0.32015.7029").holds("31.0.14005.21001"));
        assert!(!version("<", "32.0.32015.7029").holds("32.0.32015.7029"));
        assert!(version(">=", "22000").holds("26200"));
        assert!(!version("~", "1").holds("1"));
        let os = Conditions {
            os: Some(Os {
                kind: "windows".into(),
                build: Some(version(">=", "22000")),
            }),
            ..Conditions::default()
        };
        let facts = |build| Facts {
            os: "windows",
            build,
            adapter: None,
        };
        assert!(os.hold(facts(Some("26200"))));
        assert!(!os.hold(facts(Some("19045"))));
        assert!(!os.hold(facts(None)));
        // A macOS build matches with `=` as text, never with an order.
        let mac = |op: &str| Conditions {
            os: Some(Os {
                kind: "macos".into(),
                build: Some(version(op, "25C56")),
            }),
            ..Conditions::default()
        };
        let on_mac = |build| Facts {
            os: "macos",
            build: Some(build),
            adapter: None,
        };
        assert!(mac("=").hold(on_mac("25C56")));
        assert!(!mac("=").hold(on_mac("25C57")));
        assert!(!mac(">=").hold(on_mac("25C56")));
        assert_eq!(hex("0x15bf"), Some(0x15BF));
        assert_eq!(hex("nope"), None);
    }
}
