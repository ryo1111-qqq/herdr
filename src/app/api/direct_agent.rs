// Modified by ryo1111-qqq on 2026-10-09: direct creation uses the existing managed startup lifecycle.
use super::responses::{encode_error, encode_success};
use crate::api::schema::{AgentLaunchParams, ResponseResult, WorktreeInfo};
use crate::app::{
    agents::{
        valid_agent_name, AGENT_START_SETTLE_DELAY, DEFAULT_AGENT_START_TIMEOUT,
        MAX_AGENT_START_TIMEOUT,
    },
    App,
};
use crate::detect::Agent;
use crate::layout::PaneId;
use crate::terminal::TerminalState;
use std::time::{Duration, Instant};

pub(crate) struct PreparedAgentLaunch {
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    pub tab_label: Option<String>,
    name: String,
    kind: Agent,
    timeout: Duration,
}

impl PreparedAgentLaunch {
    pub(crate) fn attach(&self, terminal: &mut TerminalState) {
        terminal.begin_managed_agent(
            self.name.clone(),
            self.kind,
            Instant::now(),
            AGENT_START_SETTLE_DELAY,
            self.timeout,
        );
        if let Some(session) =
            crate::agent_resume::persisted_session_from_launch_args(self.kind, &self.argv[1..])
        {
            terminal.set_managed_agent_launch_session(session);
        }
    }
}

impl App {
    pub(crate) fn prepare_direct_agent(
        &self,
        params: &AgentLaunchParams,
    ) -> Result<PreparedAgentLaunch, (String, String)> {
        let invalid = || {
            (
                "invalid_agent_launch".into(),
                "invalid structured agent launch".into(),
            )
        };
        if !valid_agent_name(&params.name) {
            return Err(invalid());
        }
        let kind = crate::detect::parse_agent_label(&params.kind).ok_or_else(invalid)?;
        let executable = crate::detect::interactive_agent_executable(kind);
        let Some(program) = params.command.first() else {
            return Err(invalid());
        };
        let path = std::path::Path::new(program);
        if (program != executable
            && !(path.is_absolute()
                && path.file_name().and_then(|v| v.to_str()) == Some(executable)))
            || params
                .command
                .iter()
                .any(|arg| arg.chars().any(char::is_control))
        {
            return Err(invalid());
        }
        let timeout = params
            .timeout_ms
            .map(Duration::from_millis)
            .unwrap_or(DEFAULT_AGENT_START_TIMEOUT);
        if timeout <= AGENT_START_SETTLE_DELAY || timeout > MAX_AGENT_START_TIMEOUT {
            return Err(invalid());
        }
        if !self.agent_name_conflicts(&params.name, "").is_empty() {
            return Err((
                "agent_duplicate_name".into(),
                "agent name already exists".into(),
            ));
        }
        Ok(PreparedAgentLaunch {
            argv: params.command.clone(),
            env: super::env::normalize_launch_env(params.env.clone())?,
            tab_label: params.tab_label.clone(),
            name: params.name.clone(),
            kind,
            timeout,
        })
    }

    pub(crate) fn created_agent_response(
        &self,
        id: String,
        ws_idx: usize,
        tab_idx: usize,
        pane_id: PaneId,
        worktree: Option<WorktreeInfo>,
    ) -> String {
        let (Some(tab), Some(pane), Some(agent)) = (
            self.tab_info(ws_idx, tab_idx),
            self.pane_info(ws_idx, pane_id),
            self.agent_info(ws_idx, pane_id),
        ) else {
            return encode_error(
                id,
                "created_metadata_unavailable",
                "created agent metadata unavailable; do not repeat creation",
            );
        };
        encode_success(
            id,
            ResponseResult::AgentCreated {
                workspace: self.workspace_info(ws_idx),
                tab,
                pane: Box::new(pane),
                agent,
                worktree,
            },
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::{AgentCreateParams, Method, Request};
    use crate::config::Config;
    use crate::workspace::Workspace;

    fn app() -> App {
        let (_, rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            rx,
            crate::api::EventHub::default(),
        );
        app.state.workspaces = vec![Workspace::test_new("existing user shell")];
        app.state.active = Some(0);
        app.state.selected = 0;
        app.state.ensure_test_terminals();
        app
    }

    fn launch(program: String) -> AgentLaunchParams {
        AgentLaunchParams {
            name: "pi-owned".into(),
            kind: "pi".into(),
            command: vec![program, "$(touch never)".into(), "a b".into()],
            env: std::collections::HashMap::from([(
                "OWNED_LITERAL".into(),
                "x;$(touch never)".into(),
            )]),
            tab_label: Some("owned job".into()),
            timeout_ms: Some(30000),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn direct_agent_new_tab_and_split_preserve_existing_terminal_and_managed_startup() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!(
            "herdr-direct-owned-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let program = dir.join("pi");
        // A synthetic noninteractive script only; no user startup files or model calls.
        std::fs::write(
            &program,
            "#!/bin/sh\nprintf 'Owned fake agent\\n'\nread value\n",
        )
        .unwrap();
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut app = app();
        let existing_id = app.state.workspaces[0].tabs[0]
            .terminal_id(app.state.workspaces[0].tabs[0].root_pane)
            .unwrap()
            .clone();
        let workspace_id = app.public_workspace_id(0);
        let response = app.handle_api_request(Request {
            id: "direct-tab".into(),
            method: Method::TabCreateAgent(AgentCreateParams {
                create: crate::api::schema::TabCreateParams {
                    workspace_id: Some(workspace_id.clone()),
                    cwd: Some(dir.display().to_string()),
                    focus: false,
                    label: Some("owned job".into()),
                    env: Default::default(),
                },
                agent: launch(program.display().to_string()),
            }),
        });
        let value: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(value["result"]["type"], "agent_created", "{response}");
        assert_eq!(value["result"]["agent"]["launch_pending"], true);
        assert!(!value["result"]["agent"]["interactive_ready"]
            .as_bool()
            .unwrap_or(false));
        assert!(app.state.terminals.contains_key(&existing_id));
        assert_eq!(app.state.workspaces[0].tabs.len(), 2);
        let pane = value["result"]["pane"]["pane_id"]
            .as_str()
            .unwrap()
            .to_string();
        let tab = value["result"]["tab"]["tab_id"].clone();
        let mut next = launch(program.display().to_string());
        next.name = "pi-second".into();
        let response = app.handle_api_request(Request {
            id: "direct-split".into(),
            method: Method::PaneSplitAgent(AgentCreateParams {
                create: crate::api::schema::PaneSplitParams {
                    workspace_id: Some(workspace_id),
                    target_pane_id: Some(pane.clone()),
                    direction: crate::api::schema::SplitDirection::Right,
                    ratio: None,
                    cwd: Some(dir.display().to_string()),
                    focus: false,
                    right_click: Default::default(),
                    env: Default::default(),
                },
                agent: next,
            }),
        });
        let second: serde_json::Value = serde_json::from_str(&response).unwrap();
        assert_eq!(second["result"]["type"], "agent_created", "{response}");
        assert_eq!(second["result"]["pane"]["tab_id"], tab);
        assert_ne!(second["result"]["pane"]["pane_id"], pane);
        assert!(app.state.terminals.contains_key(&existing_id));
        assert_eq!(app.state.workspaces[0].tabs.len(), 2);
        crate::app::api::test_support::shutdown_test_runtimes(&mut app);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn direct_agent_rejects_shell_program_invalid_argv_env_and_timeout_before_creation() {
        let app = app();
        assert!(app.prepare_direct_agent(&launch("/bin/sh".into())).is_err());
        let mut spec = launch("pi".into());
        spec.command.push("bad\narg".into());
        assert!(app.prepare_direct_agent(&spec).is_err());
        let mut spec = launch("pi".into());
        spec.command.clear();
        assert!(app.prepare_direct_agent(&spec).is_err());
        let mut spec = launch("pi".into());
        spec.name = "Bad Name".into();
        assert!(app.prepare_direct_agent(&spec).is_err());
        let mut spec = launch("pi".into());
        spec.env.insert("BAD=KEY".into(), "value".into());
        assert!(app.prepare_direct_agent(&spec).is_err());
        let mut spec = launch("pi".into());
        spec.timeout_ms = Some(1);
        assert!(app.prepare_direct_agent(&spec).is_err());
        assert_eq!(app.state.workspaces.len(), 1);
        assert_eq!(app.state.workspaces[0].tabs.len(), 1);
    }
}
