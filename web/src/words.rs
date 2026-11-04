//! The questions homedit asks before something that can't be taken back, in the kit's confirm dialog.

use kit_rules::dialogs::Confirm;

/// Before signing out.
pub fn sign_out_question() -> Confirm {
    Confirm::new("Sign out?")
        .text("This browser forgets its sign-in.")
        .ok("Sign out")
        .danger(true)
}

/// Before leaving edit mode with unsaved changes.
pub fn discard_question() -> Confirm {
    Confirm::new("Discard unsaved changes?")
        .text("Your edits since the last save are lost.")
        .ok("Discard")
        .danger(true)
}

/// Before deleting a group with its cards.
pub fn delete_group_question(name: &str, cards: usize) -> Confirm {
    Confirm::new(format!("Delete {name}?"))
        .text(format!(
            "The group and its {} go. Undo brings them back until you leave edit mode.",
            crate::fmt::count(cards, "card", "cards")
        ))
        .ok("Delete")
        .danger(true)
}

/// Before deleting a bookmark list.
pub fn delete_list_question(name: &str) -> Confirm {
    Confirm::new(format!("Delete {name}?"))
        .text("The list and its bookmarks go. Undo brings them back.")
        .ok("Delete")
        .danger(true)
}

/// Before restoring an older version.
pub fn restore_question(version: &str) -> Confirm {
    Confirm::new(format!("Restore version {version}?"))
        .text("The dashboard goes back to this version. The current one stays in the history.")
        .ok("Restore")
}

/// Before deleting an account.
pub fn delete_user_question(name: &str) -> Confirm {
    Confirm::new(format!("Delete {name}?"))
        .text("The account can no longer sign in. This can't be undone.")
        .ok("Delete")
        .danger(true)
}

/// Before importing over the dashboard.
pub fn import_question() -> Confirm {
    Confirm::new("Import these files?")
        .text("The import replaces the dashboard. The current one stays in the history, so you can restore it.")
        .ok("Import")
}

#[cfg(test)]
mod tests {
    #[test]
    fn questions() {
        let q = super::delete_group_question("Media", 3);
        assert_eq!(
            (q.title.as_str(), q.ok.as_str(), q.danger),
            ("Delete Media?", "Delete", true)
        );
        assert!(q.text.contains("3 cards"));
        assert!(!super::restore_question("4").danger);
    }
}
