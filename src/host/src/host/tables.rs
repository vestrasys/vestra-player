//! The binding table and the command registry belong to the core; the
//! interface reads read-only snapshots of them (rule R8 of
//! doc/architecture/core-thread.md; plan step S2c(2)).
//!
//! **Who writes, who reads.** The core owns `Core::commands` and the binding
//! table here, which it writes only through [`Core::bindings_mut`]: every
//! request handler (`commands.*`, `bindings.*`, `commands.list` and
//! `commands.run`), the supervisor's park, wake and prune, the start-up
//! overlay and the registry's disabled set read and write them there. The
//! interface reads only its copies: [`Host::registry`] (menus, the palette
//! it shows, the bottom bar, the tab keys, the video click) and the
//! binding table in `InputShared::table` (the key and mouse callbacks,
//! menus, the bar, the title strip).
//!
//! **How a snapshot reaches the interface.** [`Core::table_snapshots`] is
//! the core's side: a whole copy of each table that changed since the last
//! call (the registry when its revision moved, the binding table after any
//! `bindings_mut`), as [`UiSnapshot`]s; several changes in between collapse
//! into the newest (last value wins, R3). [`Host::sync_tables`] is the
//! interface's side: it applies them, replacing what it had. Until plan
//! step S3a both run on this thread, so the interface takes them in place,
//! right where it reads: at the start of the readers that take `&mut
//! self` (`refresh_menu`, `bar_tick` (again after its own write of the
//! host's commands), `tab_keys`, `click_tick`, `window_tick` for its title
//! strip, the loop's menu step); the readers that take `&self` rely on
//! their caller having synced (`palette_items` is called only from
//! `refresh_menu`, through `refresh_palette`; the bar's items only from
//! `bar_tick`; `bind_shell` only at start-up, right after the sync there);
//! and at the end of each tick and of start-up, before Slint runs the key
//! and mouse callbacks again. A reader therefore sees the tables exactly
//! as the core left them, as it did when it read the core's own: order
//! and timing are unchanged. From
//! S3a on the core sends them on its queue to the UI thread at the end of
//! its turn, coalesced in place (module `messages`); a key event behind a
//! table update in that queue is judged by the updated table (R8).
//!
//! **Copied, then shared.** The core's tables keep changing, so the
//! interface's must be copies: one per change the interface sees, made
//! here (`Registry::view`, `Table::clone`). Each copy is shared (`Arc`)
//! from then on: the message and every holder on the interface side point
//! to the same value, and from S3a it crosses the thread without a second
//! copy. Measured in plan step S2c(2) (release build, offscreen, the 14
//! built-in plugins, 45 s with a clip): 336 commands, a registry copy of
//! about 120 kB (as JSON) and a table of 336 declarations, each copied in
//! 0.1–0.9 ms. Of 529 registry revisions and about 370 table changes, 28
//! and 37 reached the interface as copies (several changes between two
//! reads collapse), 23 ms in all, every one at start-up or when plugins
//! parked; none while the clip played.
use super::messages::{UiCommand, UiSnapshot};
use super::*;

/// The core's binding table and what the interface was last given.
#[derive(Default)]
pub(crate) struct Tables {
    /// The binding table: command declarations, the user's overlay and the
    /// registry's disabled set.
    bindings: bindings::Table,
    /// Moves on every [`Core::bindings_mut`].
    changes: u64,
    /// What the interface was last given: the binding table's `changes`
    /// and the registry's revision (`None`: nothing yet).
    sent: (Option<u64>, Option<u64>),
}

impl Core {
    /// The binding table, for the core's readers.
    pub(crate) fn bindings(&self) -> &bindings::Table {
        &self.tables.bindings
    }
    /// The binding table, for a writer: the interface gets a new copy
    /// before it next reads one (`Host::sync_tables`).
    pub(crate) fn bindings_mut(&mut self) -> &mut bindings::Table {
        self.tables.changes += 1;
        &mut self.tables.bindings
    }
    /// The snapshots of the tables that changed since the last call (rule
    /// R8): the command registry's when its revision moved (every change
    /// the interface reads moves it, as for its menu tree), the binding
    /// table's after a write.
    pub(crate) fn table_snapshots(&mut self) -> Vec<UiSnapshot> {
        let mut snapshots = Vec::new();
        let revision = self.commands.revision();
        if self.tables.sent.1 != Some(revision) {
            self.tables.sent.1 = Some(revision);
            snapshots.push(UiSnapshot::Registry(Arc::new(self.commands.view())));
        }
        if self.tables.sent.0 != Some(self.tables.changes) {
            self.tables.sent.0 = Some(self.tables.changes);
            snapshots.push(UiSnapshot::Bindings(Arc::new(self.tables.bindings.clone())));
        }
        snapshots
    }
}

impl Host {
    /// The interface takes the core's newer table snapshots, in place
    /// (module docs): what it reads from here on is what the core has.
    pub(crate) fn sync_tables(&mut self) {
        // Asked of the core (plan step S3a(1), `Take::Tables`).
        for command in self.take(super::messages::Take::Tables) {
            match command {
                UiCommand::Snapshot(UiSnapshot::Registry(view)) => self.registry = view,
                UiCommand::Snapshot(UiSnapshot::Bindings(table)) => {
                    self.input.borrow_mut().table = table.into()
                }
                _ => unreachable!("the core answers with its tables only"),
            }
        }
    }
}
