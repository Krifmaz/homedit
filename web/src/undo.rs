//! Undo and redo for the editor: snapshots of the document. Edits that share a coalesce key (typing in one
//! field) collapse into one step.

use crate::model::Doc;

/// Undo steps kept.
pub const MAX_UNDO: usize = 100;

/// The stacks around the document being edited.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Undo {
    /// Earlier snapshots, oldest first.
    pub past: Vec<Doc>,
    /// The document being edited.
    pub present: Doc,
    /// Undone snapshots, the most recently undone last.
    pub future: Vec<Doc>,
    /// The snapshot last saved to (or loaded from) the server.
    pub saved: Doc,
    /// The last edit's key.
    pub last_key: Option<String>,
}

impl Undo {
    /// Fresh stacks for `doc`.
    pub fn new(doc: Doc) -> Undo {
        Undo {
            past: vec![],
            present: doc.clone(),
            future: vec![],
            saved: doc,
            last_key: None,
        }
    }

    /// Replaces the document with `next`; with the same `key` as the last edit, in the same step.
    pub fn apply(&mut self, next: Doc, key: Option<&str>) {
        if next == self.present {
            return;
        }
        if key.is_some() && key.map(str::to_owned) == self.last_key && !self.past.is_empty() {
            self.present = next;
            self.future.clear();
            return;
        }
        self.past.push(std::mem::replace(&mut self.present, next));
        if self.past.len() > MAX_UNDO {
            self.past.remove(0);
        }
        self.future.clear();
        self.last_key = key.map(str::to_owned);
    }

    /// Goes one step back.
    pub fn undo(&mut self) {
        if let Some(prev) = self.past.pop() {
            self.future.push(std::mem::replace(&mut self.present, prev));
            self.last_key = None;
        }
    }

    /// Goes one step forward.
    pub fn redo(&mut self) {
        if let Some(next) = self.future.pop() {
            self.past.push(std::mem::replace(&mut self.present, next));
            self.last_key = None;
        }
    }

    /// The save went through as `version`: every snapshot carries it, so a later save of any of them is not
    /// refused as stale.
    pub fn saved_as(&mut self, version: &str) {
        for d in self
            .past
            .iter_mut()
            .chain(self.future.iter_mut())
            .chain(std::iter::once(&mut self.present))
        {
            d.version = version.to_string();
        }
        self.saved = self.present.clone();
        self.last_key = None;
    }

    /// The server's copy of what was just saved (with the ids it gave new items).
    pub fn synced(&mut self, doc: Doc) {
        self.present = doc.clone();
        self.saved = doc;
        self.last_key = None;
    }

    /// Whether the document differs from the saved one.
    pub fn dirty(&self) -> bool {
        self.present != self.saved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titled(t: &str) -> Doc {
        let mut d = Doc::default();
        d.settings.title = t.into();
        d
    }

    #[test]
    fn undo_redo_and_dirty() {
        let mut u = Undo::new(titled("a"));
        assert!(!u.dirty());
        u.apply(titled("b"), None);
        u.apply(titled("c"), None);
        assert!(u.dirty());
        u.undo();
        assert_eq!(u.present.settings.title, "b");
        u.redo();
        assert_eq!(u.present.settings.title, "c");
        u.undo();
        u.undo();
        assert!(!u.dirty());
        u.undo();
        assert_eq!(u.present.settings.title, "a");
        u.apply(titled("z"), None);
        assert!(u.future.is_empty());
    }

    #[test]
    fn same_key_edits_are_one_step() {
        let mut u = Undo::new(titled(""));
        u.apply(titled("h"), Some("title"));
        u.apply(titled("he"), Some("title"));
        u.apply(titled("hey"), Some("title"));
        assert_eq!(u.past.len(), 1);
        u.apply(titled("hey!"), Some("other"));
        assert_eq!(u.past.len(), 2);
        u.undo();
        u.undo();
        assert_eq!(u.present.settings.title, "");
        // Same document again: no step.
        u.apply(titled(""), None);
        assert!(u.past.is_empty());
    }

    #[test]
    fn keeps_at_most_max_steps() {
        let mut u = Undo::new(titled("0"));
        for i in 1..=MAX_UNDO + 5 {
            u.apply(titled(&i.to_string()), None);
        }
        assert_eq!(u.past.len(), MAX_UNDO);
    }

    #[test]
    fn saving_stamps_every_snapshot() {
        let mut u = Undo::new(titled("a"));
        u.apply(titled("b"), None);
        u.saved_as("7");
        assert!(!u.dirty());
        assert_eq!(u.past[0].version, "7");
        u.undo();
        assert!(u.dirty());
        assert_eq!(u.present.version, "7");
        u.synced(titled("server"));
        assert!(!u.dirty());
    }
}
