//! A value with a revision that moves on every change: `packages.changed`
//! rebuilds `packages.list` only when one of its sources moved
//! (`binding_manager.rs` `notify_tick`). Reading goes
//! through `Deref`; changing only through `edit`, so a change that skips the
//! revision does not compile.
#![forbid(unsafe_code)]

/// `T` and how many times it was opened for a change.
#[derive(Debug, Default)]
pub(crate) struct Revised<T> {
    value: T,
    revision: u64,
}

impl<T> Revised<T> {
    /// The value, for a change: the revision moves (whether or not anything
    /// is changed then, the safe side).
    pub(crate) fn edit(&mut self) -> &mut T {
        self.revision += 1;
        &mut self.value
    }
    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }
}

impl<T> std::ops::Deref for Revised<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn reading_keeps_the_revision_and_every_edit_moves_it() {
        let mut failures = Revised::<BTreeMap<String, String>>::default();
        assert_eq!(failures.revision(), 0);
        assert!(failures.get("a.b").is_none());
        assert_eq!(failures.revision(), 0);
        failures.edit().insert("a.b".into(), "crashed".into());
        assert_eq!(failures.revision(), 1);
        assert_eq!(failures.len(), 1);
        // Opened for a change that changes nothing: moved all the same.
        failures.edit().remove("c.d");
        assert_eq!(failures.revision(), 2);
    }
}
