//! The process budget's input and the memory pressure source
//! (doc/architecture/supervisor.md). The budget is the core's
//! (`Tunables::new` from the physical memory read once at start-up); the pressure level is read from the system every
//! `POLL_MS` (Windows: thresholds on available physical memory and
//! the system's low-memory notification; Linux: PSI; macOS: none yet,
//! `platform::PressureSource`) and given to the core when it changes
//! (`memory_pressure`). Both put idle third-party plugins in
//! standby through the same Standby job as the idle timer.
//!
//! Test switches (any build; isolated profile, like `probe.rs`'s):
//!
//! - `VESTRA_TEST_PHYS_MIB=<MiB>`: the physical memory the budget formula
//!   reads instead of the machine's (a small budget with few plugins).
//! - `VESTRA_TEST_PRESSURE=<none|low|critical>@<ms>[,...]`: the pressure
//!   level from that many milliseconds after the host started, instead of
//!   the system's (which is then not read at all): a pressure signal
//!   without putting the machine under memory pressure.
use super::*;
use crate::platform::{self, MemoryPressure};
use crate::supervisor::Pressure;

/// How often the system's level is read: every 5 s.
pub(super) const POLL_MS: u64 = 5_000;

/// The pressure source and the level the core has.
#[derive(Default)]
pub(crate) struct Memory {
    /// Opened at the first poll.
    source: Option<platform::PressureSource>,
    /// What the core was last told.
    level: Pressure,
    next_poll: u64,
    /// `VESTRA_TEST_PRESSURE`, by time; when set, the system is not read.
    tests: Vec<(u64, Pressure)>,
    simulated: bool,
}
impl Memory {
    pub fn from_env() -> Self {
        let tests = std::env::var("VESTRA_TEST_PRESSURE")
            .map(|text| test_pressures(&text))
            .unwrap_or_default();
        Self {
            simulated: !tests.is_empty(),
            tests,
            ..Self::default()
        }
    }
}

/// The physical memory the budget formula reads: `VESTRA_TEST_PHYS_MIB`,
/// else the machine's (8 GiB when the system does not say).
pub(super) fn physical_mib() -> u64 {
    std::env::var("VESTRA_TEST_PHYS_MIB")
        .ok()
        .and_then(|mib| mib.parse().ok())
        .filter(|mib| *mib > 0)
        .unwrap_or_else(platform::physical_memory_mib)
}

fn level_of(pressure: MemoryPressure) -> Pressure {
    match pressure {
        MemoryPressure::None => Pressure::None,
        MemoryPressure::Low => Pressure::Low,
        MemoryPressure::Critical => Pressure::Critical,
    }
}

/// `VESTRA_TEST_PRESSURE` entries, in time order; malformed ones skipped.
fn test_pressures(text: &str) -> Vec<(u64, Pressure)> {
    let mut entries = text
        .split(',')
        .filter_map(|entry| {
            let (level, at) = entry.trim().split_once('@')?;
            let level = match level {
                "none" => Pressure::None,
                "low" => Pressure::Low,
                "critical" => Pressure::Critical,
                _ => return None,
            };
            Some((at.parse().ok()?, level))
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|(at, _)| *at);
    entries
}

impl Host {
    /// Every host tick: every `POLL_MS` the
    /// system's pressure level (or the test switch's); a change goes to
    /// the core, which puts idle third-party plugins in standby for Low and
    /// Critical.
    pub(super) fn memory_tick(&mut self, now: u64) {
        let memory = &mut self.core.on_demand.memory;
        let level = if memory.simulated {
            let due = memory.tests.iter().take_while(|(at, _)| *at <= now).count();
            match memory.tests.drain(..due).next_back() {
                Some((_, level)) => level,
                None => return,
            }
        } else {
            if now < memory.next_poll {
                return;
            }
            memory.next_poll = now + POLL_MS;
            let source = memory
                .source
                .get_or_insert_with(platform::PressureSource::new);
            match source.level() {
                Some(level) => level_of(level),
                None => return,
            }
        };
        if level == memory.level {
            return;
        }
        memory.level = level;
        diag!(
            "supervisor: memory pressure {level:?}{} ({} third-party processes)",
            if memory.simulated {
                " (VESTRA_TEST_PRESSURE)"
            } else {
                ""
            },
            self.core.supervisor.budget_count()
        );
        self.core.supervisor.set_now(now);
        self.core.supervisor.memory_pressure(level);
        self.supervisor_run();
    }
}
impl Core {
    /// Why the core asks `owner` for its resources, when that is the
    /// process budget or memory pressure (a log line; the idle timer's
    /// asks are not logged).
    pub(super) fn log_reclaim(&self, owner: u64) {
        let Some(unit) = self.supervisor.by_pid(Pid(owner)) else {
            return;
        };
        // The job's origin says which: a budget standby under Low pressure
        // is still the budget's.
        let why = match self.supervisor.job(unit).map(|job| job.origin) {
            Some(crate::supervisor::Origin::Budget) => format!(
                "over the process budget ({} of {} third-party processes)",
                self.supervisor.budget_count(),
                self.supervisor.tunables().process_budget
            ),
            Some(crate::supervisor::Origin::Pressure) => {
                format!("memory pressure {:?}", self.supervisor.pressure())
            }
            _ => return,
        };
        diag!(
            "supervisor: {} to standby if idle: {why}",
            self.plugin_id(owner)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pressures_parse_in_time_order() {
        assert_eq!(
            test_pressures("critical@9000, low@3000,none@20000"),
            [
                (3_000, Pressure::Low),
                (9_000, Pressure::Critical),
                (20_000, Pressure::None)
            ]
        );
        assert!(test_pressures("").is_empty());
        assert_eq!(test_pressures("high@1,low@x,low@5"), [(5, Pressure::Low)]);
        assert!(!Memory::default().simulated);
    }

    #[test]
    fn system_levels_map_to_the_cores() {
        assert_eq!(level_of(MemoryPressure::None), Pressure::None);
        assert_eq!(level_of(MemoryPressure::Low), Pressure::Low);
        assert_eq!(level_of(MemoryPressure::Critical), Pressure::Critical);
    }
}
