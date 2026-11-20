// SPDX-License-Identifier: GPL-3.0-or-later
//! Native window tabs are independent of each shell's reader-tab session.
#![forbid(unsafe_code)]
use crate::store::WindowSession;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
// Only the macOS window-tab snapshot constructs members.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) struct Member {
    pub label: String,
    pub group: String,
    pub index: usize,
    pub selected: bool,
    pub focused: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Group {
    pub windows: Vec<String>,
    pub selected: String,
    pub focused: bool,
}

/// Missing windows (including failed creations) cannot leave dangling native
/// selection. Legacy sessions have no group metadata and keep AppKit defaults.
pub(super) fn restore_plan(
    sessions: &HashMap<String, WindowSession>,
    open: &HashSet<String>,
) -> Vec<Group> {
    let mut groups: BTreeMap<&str, Vec<(&str, &crate::store::NativeWindowTab)>> = BTreeMap::new();
    for (label, session) in sessions {
        if !open.contains(label) || !shell_label(label) {
            continue;
        }
        if let Some(tab) = &session.native_tab {
            if !tab.group.is_empty() {
                groups.entry(&tab.group).or_default().push((label, tab));
            }
        }
    }
    let mut focused = false;
    groups.into_values().map(|mut members| {
        // The label tie-breaker makes malformed duplicate positions deterministic.
        members.sort_by(|(left, a), (right, b)| a.index.cmp(&b.index).then(left.cmp(right)));
        let selected = members.iter().find(|(_, tab)| tab.selected)
            .unwrap_or(&members[0]).0.to_string();
        let group_focused = !focused && members.iter().any(|(_, tab)| tab.focused);
        focused |= group_focused;
        Group {
            windows: members.into_iter().map(|(label, _)| label.to_string()).collect(),
            selected,
            focused: group_focused,
        }
    }).collect()
}

pub(super) fn shell_label(label: &str) -> bool {
    label == "main" || label.starts_with("window-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::NativeWindowTab;

    fn session(group: &str, index: usize, selected: bool, focused: bool) -> WindowSession {
        WindowSession { native_tab: Some(NativeWindowTab {
            group: group.into(), index, selected, focused,
        }), ..WindowSession::default() }
    }

    #[test]
    fn preserves_native_groups_order_selection_and_focus_without_reader_tabs() {
        let sessions = HashMap::from([
            ("main".into(), session("first", 1, true, true)),
            ("window-a".into(), session("first", 0, false, false)),
            ("window-b".into(), session("second", 0, true, false)),
        ]);
        let groups = restore_plan(&sessions, &sessions.keys().cloned().collect());
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].windows, ["window-a", "main"]);
        assert_eq!(groups[0].selected, "main");
        assert!(groups[0].focused);
        assert_eq!(groups[1].windows, ["window-b"]);
        assert_eq!(groups[1].selected, "window-b");
        assert!(!groups[1].focused);
    }

    #[test]
    fn failed_creation_reselects_survivor_and_unrelated_live_windows_stay_untouched() {
        let sessions = HashMap::from([
            ("main".into(), session("group", 0, true, true)),
            ("window-a".into(), session("group", 1, false, false)),
        ]);
        let open = HashSet::from(["window-a".into(), "window-live".into()]);
        let groups = restore_plan(&sessions, &open);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].windows, ["window-a"]);
        assert_eq!(groups[0].selected, "window-a");
    }

    #[test]
    fn legacy_sessions_and_non_shell_windows_do_not_authorize_regrouping() {
        let sessions = HashMap::from([
            ("main".into(), WindowSession::default()),
            ("preferences".into(), session("group", 0, true, true)),
        ]);
        assert!(restore_plan(&sessions, &sessions.keys().cloned().collect()).is_empty());
    }
}
