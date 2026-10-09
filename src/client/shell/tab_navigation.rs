// Modified in this fork: shared worktree tabs and close confirmation.
use super::{ClientShellSnapshot, ClientShellTab, HashMap};

fn group_members(snapshot: &ClientShellSnapshot) -> Option<HashMap<&str, (bool, usize)>> {
    let focused = snapshot.focused_workspace_id.as_deref();
    let group_key = snapshot
        .workspaces
        .iter()
        .find(|workspace| Some(workspace.workspace_id.as_str()) == focused)
        .and_then(|workspace| workspace.worktree.as_ref())
        .map(|worktree| worktree.key.as_str())
        .filter(|key| !key.is_empty());
    let key = group_key?;
    Some(
        snapshot
            .workspaces
            .iter()
            .enumerate()
            .filter_map(|(index, workspace)| {
                let worktree = workspace.worktree.as_ref().filter(|tree| tree.key == key)?;
                Some((
                    workspace.workspace_id.as_str(),
                    (worktree.is_linked_worktree, index),
                ))
            })
            .collect::<HashMap<_, _>>(),
    )
}

pub(super) fn visible_tab_count(snapshot: &ClientShellSnapshot) -> usize {
    let members = group_members(snapshot);
    snapshot
        .tabs
        .iter()
        .filter(|tab| match &members {
            Some(members) => members.contains_key(tab.workspace_id.as_str()),
            None => Some(tab.workspace_id.as_str()) == snapshot.focused_workspace_id.as_deref(),
        })
        .count()
}

/// A presentation of existing tabs; their runtime ownership never changes.
pub(super) fn visible_tabs(snapshot: &ClientShellSnapshot) -> Vec<&ClientShellTab> {
    let Some(members) = group_members(snapshot) else {
        return snapshot
            .tabs
            .iter()
            .filter(|tab| {
                Some(tab.workspace_id.as_str()) == snapshot.focused_workspace_id.as_deref()
            })
            .collect();
    };
    let mut tabs = snapshot
        .tabs
        .iter()
        .filter(|tab| members.contains_key(tab.workspace_id.as_str()))
        .collect::<Vec<_>>();
    // Stable sorting preserves native tab order inside each Space.
    tabs.sort_by_key(|tab| members[tab.workspace_id.as_str()]);
    tabs
}
