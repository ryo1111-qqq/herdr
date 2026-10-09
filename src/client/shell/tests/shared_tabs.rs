// Modified in this fork: shared worktree tabs and close confirmation.
use super::*;
use crate::api::schema::Method;
use crate::input::KeybindAction;

fn grouped_snapshot() -> ClientShellSnapshot {
    let mut projection = snapshot();
    let workspace = projection.workspaces[0].clone();
    let tab = projection.tabs[0].clone();
    projection.workspaces = [
        ("job-a", "repo", true),
        ("other", "unrelated-repo", false),
        ("main", "repo", false),
        ("job-b", "repo", true),
    ]
    .iter()
    .enumerate()
    .map(|(index, (id, key, linked))| ClientShellWorkspace {
        workspace_id: (*id).into(),
        active_tab_id: format!("{id}:1"),
        label: (*id).into(),
        number: index + 1,
        focused: *id == "job-a",
        worktree: Some(ClientShellWorktree {
            key: (*key).into(),
            label: (*key).into(),
            is_linked_worktree: *linked,
        }),
        ..workspace.clone()
    })
    .collect();
    projection.tabs = ["job-a", "job-a", "other", "main", "job-b"]
        .iter()
        .enumerate()
        .map(|(index, id)| ClientShellTab {
            workspace_id: (*id).into(),
            tab_id: if index == 1 {
                "job-a:2".into()
            } else {
                format!("{id}:1")
            },
            label: if *id == "main" {
                "orch".into()
            } else {
                format!("{id}-{index}")
            },
            focused: index == 0,
            ..tab.clone()
        })
        .collect();
    projection.focused_workspace_id = Some("job-a".into());
    projection.focused_tab_id = Some("job-a:1".into());
    projection.panes[0].workspace_id = "job-a".into();
    projection.panes[0].tab_id = "job-a:1".into();
    projection.panes[0].cwd = Some("/repo-worktree-a".into());
    projection
}

fn state_for(projection: ClientShellSnapshot) -> ClientShellState {
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    let mut frame = surface();
    frame.panes[0].pane_id = projection.focused_pane_id.clone().expect("populated pane");
    state.set_snapshot(Box::new(projection));
    state.set_pane_surface(frame);
    state
}

fn tab_ids(state: &ClientShellState) -> Vec<&str> {
    state.hits.tabs.iter().map(|(_, id)| id.as_str()).collect()
}

#[test]
fn shared_tabs_keep_main_first_and_other_repository_out_without_mutating_ownership() {
    let projection = grouped_snapshot();
    let mut state = state_for(projection.clone());
    state.compose(160, 30).expect("grouped tab bar");
    assert_eq!(tab_ids(&state), ["main:1", "job-a:1", "job-a:2", "job-b:1"]);
    assert_eq!(state.snapshot.as_deref(), Some(&projection));
}

#[test]
fn shared_tabs_identify_the_preserved_worktree_root_beside_its_job() {
    let mut projection = grouped_snapshot();
    projection.tabs[0].label = "1".into();
    projection.tabs[1].label = "Issue #173 Claude".into();
    projection.tabs[1].custom_label = true;
    let mut state = state_for(projection.clone());
    let frame = state.compose(160, 30).expect("root and job tabs");
    let text = frame_rows(&frame).join("\n");
    assert!(
        text.contains("job-a · 1"),
        "worktree root needs its owner label"
    );
    assert!(text.contains("job-a · Issue #173 Claude"));
    assert_eq!(state.snapshot.as_deref(), Some(&projection));
}

#[test]
fn shared_tabs_group_close_warns_about_unfinished_work_and_keeps_confirmation_scope() {
    let mut state = state_for(grouped_snapshot());
    state.open_confirm_close_overlay("main".into());
    let frame = state.compose(160, 30).expect("group confirmation");
    let text = frame_rows(&frame).join("\n");
    assert!(text.contains("Close worktree group?"));
    assert!(text.contains("3 workspaces, 1 pane"));
    assert!(text.contains("Unfinished work will stop. Git files will remain."));
    let mut outcome = ClientShellInput::default();
    state.accept_close_confirmation(&mut outcome);
    assert!(
        matches!(&outcome.actions[..], [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, Method::WorkspaceClose(params)
            if params.workspace_id == "main" && params.close_group))
    );
}

#[test]
fn shared_tabs_click_and_keyboard_target_the_owning_tab_and_create_locally() {
    let mut state = state_for(grouped_snapshot());
    state.compose(160, 30).expect("grouped tab bar");
    let parent = state
        .hits
        .tabs
        .iter()
        .find(|(_, id)| id == "main:1")
        .expect("orch tab")
        .0;
    let outcome = state.handle_raw_events(vec![
        RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: parent.x + 1,
            row: parent.y,
            modifiers: KeyModifiers::empty(),
        }),
        RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: parent.x + 1,
            row: parent.y,
            modifiers: KeyModifiers::empty(),
        }),
    ]);
    assert!(
        matches!(&outcome.actions[..], [ClientShellAction::Endpoint { request, .. }]
        if matches!(&request.method, Method::TabFocus(target) if target.tab_id == "main:1"))
    );
    for (action, id) in [
        (KeybindAction::SwitchTab(0), "main:1"),
        (KeybindAction::SwitchTab(3), "job-b:1"),
        (KeybindAction::PreviousTab, "main:1"),
        (KeybindAction::NextTab, "job-a:2"),
    ] {
        assert!(
            matches!(state.endpoint_method_for_action(action), Some(Method::TabFocus(target))
            if target.tab_id == id)
        );
    }
    state.config.prompt_new_tab_name = false;
    assert!(
        matches!(state.endpoint_method_for_action(KeybindAction::NewTab),
        Some(Method::TabCreate(params)) if params.workspace_id.as_deref() == Some("job-a"))
    );
    assert!(
        matches!(state.endpoint_method_for_action(KeybindAction::MoveTabNext),
        Some(Method::TabMove(params)) if params.tab_id == "job-a:1" && params.insert_index == 2)
    );
}

#[test]
fn shared_tabs_single_tab_setting_counts_the_group_and_missing_membership_stays_local() {
    let mut projection = grouped_snapshot();
    projection.tabs.retain(|t| t.tab_id != "job-a:2");
    let mut config = ClientShellConfig::from_config(&Config::default());
    config.hide_tab_bar_when_single_tab = true;
    let mut state = ClientShellState::new(config);
    state.set_snapshot(Box::new(projection.clone()));
    state.set_pane_surface(surface());
    state
        .compose(160, 30)
        .expect("one local tab but three group tabs");
    assert_eq!(tab_ids(&state), ["main:1", "job-a:1", "job-b:1"]);
    projection.workspaces[0].worktree = None;
    state.set_snapshot(Box::new(projection));
    state.set_pane_surface(surface());
    state.compose(160, 30).expect("ungrouped Space");
    assert!(state.hits.tabs.is_empty());
}

#[test]
fn shared_tabs_drag_to_another_space_does_not_move_a_tab() {
    let mut state = state_for(grouped_snapshot());
    state.compose(160, 30).expect("grouped tab bar");
    let job = state
        .hits
        .tabs
        .iter()
        .find(|(_, id)| id == "job-a:1")
        .expect("job tab")
        .0;
    let other = state
        .hits
        .tabs
        .iter()
        .find(|(_, id)| id == "job-b:1")
        .expect("other Space tab")
        .0;
    let outcome = state.handle_raw_events(vec![
        RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: job.x + 1,
            row: job.y,
            modifiers: KeyModifiers::empty(),
        }),
        RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: other.x + 1,
            row: other.y,
            modifiers: KeyModifiers::empty(),
        }),
        RawInputEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: other.x + 1,
            row: other.y,
            modifiers: KeyModifiers::empty(),
        }),
    ]);
    assert!(!outcome.actions.iter().any(|action| matches!(action,
        ClientShellAction::Endpoint { request, .. } if matches!(request.method, Method::TabMove(_)))));
}

fn overflowing_family(focused: &str) -> ClientShellSnapshot {
    let mut projection = grouped_snapshot();
    projection
        .tabs
        .retain(|tab| tab.tab_id != "job-a:2" && tab.workspace_id != "other");
    let template = projection.tabs[0].clone();
    projection.tabs.extend((2..=9).map(|number| ClientShellTab {
        tab_id: format!("job-a:{number}"),
        workspace_id: "job-a".into(),
        label: format!("long sibling task {number}"),
        focused: false,
        ..template.clone()
    }));
    projection.focused_workspace_id = Some(focused.into());
    projection.focused_tab_id = Some(format!("{focused}:1"));
    for tab in &mut projection.tabs {
        tab.focused = Some(tab.tab_id.as_str()) == projection.focused_tab_id.as_deref();
    }
    projection
}

fn click_scroll(state: &mut ClientShellState, right: bool) {
    let rect = if right {
        state.hits.tab_scroll_right
    } else {
        state.hits.tab_scroll_left
    };
    assert!(!rect.is_empty());
    let outcome = state.handle_raw_events(vec![RawInputEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x + 1,
        row: rect.y,
        modifiers: KeyModifiers::empty(),
    })]);
    assert!(outcome.repaint);
    assert!(
        outcome.actions.is_empty(),
        "scrolling only changes client presentation"
    );
}

#[test]
fn shared_tabs_overflow_scroll_reaches_siblings_from_single_tab_parent_and_child() {
    for focused in ["main", "job-b"] {
        let projection = overflowing_family(focused);
        assert_eq!(
            projection
                .tabs
                .iter()
                .filter(|t| t.workspace_id == focused)
                .count(),
            1
        );
        let mut state = state_for(projection.clone());
        state.config.mobile_width_threshold = 0;
        state
            .compose(90, 20)
            .expect("initial geometry reveals selected tab");
        state.tab_scroll = 0;
        state.reveal_focused_tab = false;
        state
            .compose(90, 20)
            .expect("manually scrolled family strip");
        assert!(!tab_ids(&state).contains(&"job-b:1"));
        click_scroll(&mut state, true);
        state.compose(90, 20).expect("shared scroll right");
        assert_eq!(
            state.tab_scroll, 1,
            "one local tab must not clamp the family to zero"
        );
        for _ in 0..projection.tabs.len() {
            if tab_ids(&state).contains(&"job-b:1") {
                break;
            }
            click_scroll(&mut state, true);
            state.compose(90, 20).expect("reveal final sibling");
        }
        assert!(tab_ids(&state).contains(&"job-b:1"));
        let end = state.tab_scroll;
        click_scroll(&mut state, false);
        state.compose(90, 20).expect("shared scroll left");
        assert_eq!(state.tab_scroll, end - 1);
        click_scroll(&mut state, true);
        state.compose(90, 20).expect("return to final sibling");
        assert_eq!(state.tab_scroll, end);
        assert_eq!(state.snapshot.as_deref(), Some(&projection));
    }
}

#[test]
fn shared_tabs_keyboard_focus_preserves_subsequent_left_right_scroll() {
    let mut state = state_for(overflowing_family("main"));
    state.config.mobile_width_threshold = 0;
    state.compose(90, 20).expect("initial family strip");
    assert!(
        matches!(state.endpoint_method_for_action(KeybindAction::SwitchTab(10)),
        Some(Method::TabFocus(target)) if target.tab_id == "job-b:1")
    );
    state.set_snapshot(Box::new(overflowing_family("job-b")));
    state.compose(90, 20).expect("keyboard-selected sibling");
    assert!(tab_ids(&state).contains(&"job-b:1"));
    let selected_scroll = state.tab_scroll;
    assert!(selected_scroll > 1);
    click_scroll(&mut state, false);
    state.compose(90, 20).expect("manual left after keyboard");
    assert_eq!(state.tab_scroll, selected_scroll - 1);
    click_scroll(&mut state, true);
    state.compose(90, 20).expect("manual right after keyboard");
    assert_eq!(
        state.tab_scroll, selected_scroll,
        "do not reset a keyboard-revealed family strip"
    );
    assert!(tab_ids(&state).contains(&"job-b:1"));
}

#[test]
#[ignore = "fixed-geometry client render scaling profile"]
fn render_scale_profile_shared_tab_bar() {
    for count in [1, 15] {
        let mut projection = grouped_snapshot();
        let workspace = projection.workspaces[2].clone();
        let tab = projection.tabs[3].clone();
        let pane = projection.panes[0].clone();
        projection.workspaces = (0..count)
            .map(|i| ClientShellWorkspace {
                workspace_id: format!("ws-{i}"),
                active_tab_id: format!("tab-{i}"),
                focused: i == 0,
                worktree: Some(ClientShellWorktree {
                    key: "repo".into(),
                    label: "repo".into(),
                    is_linked_worktree: i != 0,
                }),
                ..workspace.clone()
            })
            .collect();
        projection.tabs = (0..count)
            .map(|i| ClientShellTab {
                workspace_id: format!("ws-{i}"),
                tab_id: format!("tab-{i}"),
                focused: i == 0,
                ..tab.clone()
            })
            .collect();
        projection.panes = (0..count)
            .map(|i| ClientShellPane {
                workspace_id: format!("ws-{i}"),
                tab_id: format!("tab-{i}"),
                pane_id: format!("pane-{i}"),
                ..pane.clone()
            })
            .collect();
        projection.focused_workspace_id = Some("ws-0".into());
        projection.focused_tab_id = Some("tab-0".into());
        projection.focused_pane_id = Some("pane-0".into());
        let mut state = state_for(projection);
        state.compose(120, 30).expect("warmup");
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            std::hint::black_box(state.compose(120, 30).expect("fixed frame"));
        }
        eprintln!(
            "shared_tab_bar panes={count} frames=1000 elapsed_us={}",
            start.elapsed().as_micros()
        );
    }
}
