use std::os::unix::net::UnixStream;

use nmux_core::host::{HostSpec, ProcessHost};
use nmux_core::session::{
    PendingSessionEvent, Session, SessionActor, SessionEvent, SessionEventLane,
};
use nmux_proto::{protocol, wire};

use super::{ControlCommandSummary, write_protocol_error};
use crate::error::ServeError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ControlCommandOutcome {
    Continue,
    Shutdown,
}

pub(super) fn serve_control_command(
    stream: &mut UnixStream,
    command: ControlCommandSummary,
    session: &mut Session,
    host: Option<&mut dyn ProcessHost>,
) -> Result<ControlCommandOutcome, ServeError> {
    let mut seq = 1;
    match apply_control_command(session, host, &command) {
        Ok(outcome) => {
            let workspace_frame = session.workspace_tree_frame("local-client", seq);
            wire::write_default_frame(stream, &workspace_frame)?;
            Ok(outcome)
        }
        Err(error) => {
            write_protocol_error(
                stream,
                session,
                &mut seq,
                error.code,
                &error.message,
                error.pane_id.as_deref(),
                command.command_seq,
            )?;
            Ok(ControlCommandOutcome::Continue)
        }
    }
}

pub(super) fn serve_control_command_with_session_actor(
    stream: &mut UnixStream,
    command: ControlCommandSummary,
    actor: &mut SessionActor,
    host: Option<&mut dyn ProcessHost>,
) -> Result<ControlCommandOutcome, ServeError> {
    let mut seq = 1;
    match apply_control_command_with_session_actor(actor, host, &command) {
        Ok(outcome) => {
            let workspace_frame = actor.session().workspace_tree_frame("local-client", seq);
            wire::write_default_frame(stream, &workspace_frame)?;
            Ok(outcome)
        }
        Err(error) => {
            write_protocol_error(
                stream,
                actor.session(),
                &mut seq,
                error.code,
                &error.message,
                error.pane_id.as_deref(),
                command.command_seq,
            )?;
            Ok(ControlCommandOutcome::Continue)
        }
    }
}

fn apply_control_command(
    session: &mut Session,
    host: Option<&mut dyn ProcessHost>,
    command: &ControlCommandSummary,
) -> Result<ControlCommandOutcome, ControlCommandError> {
    match command.kind {
        protocol::ControlCommandKind::PaneSplit => {
            let Some(host) = host else {
                return Err(ControlCommandError::unknown(
                    "pane split requires a process host",
                    command.pane_id.clone(),
                ));
            };
            apply_pane_split_command(session, host, command)?;
            Ok(ControlCommandOutcome::Continue)
        }
        protocol::ControlCommandKind::TabNew => {
            let Some(host) = host else {
                return Err(ControlCommandError::unknown(
                    "tab new requires a process host",
                    None,
                ));
            };
            apply_tab_new_command(session, host, command)?;
            Ok(ControlCommandOutcome::Continue)
        }
        protocol::ControlCommandKind::TabClose => {
            apply_tab_close_command(session, host, command)?;
            Ok(ControlCommandOutcome::Continue)
        }
        protocol::ControlCommandKind::TabSwitch => {
            apply_tab_switch_command(session, command)?;
            Ok(ControlCommandOutcome::Continue)
        }
        protocol::ControlCommandKind::SessionNew => Err(ControlCommandError::unknown(
            "session new requires daemon registry routing",
            None,
        )),
        protocol::ControlCommandKind::SessionKill => {
            apply_session_kill_command(session, command)?;
            Ok(ControlCommandOutcome::Shutdown)
        }
        _ => Err(ControlCommandError::unknown(
            format!("unknown control command kind {}", command.kind.0),
            command.pane_id.clone(),
        )),
    }
}

fn apply_control_command_with_session_actor(
    actor: &mut SessionActor,
    host: Option<&mut dyn ProcessHost>,
    command: &ControlCommandSummary,
) -> Result<ControlCommandOutcome, ControlCommandError> {
    match command.kind {
        protocol::ControlCommandKind::PaneSplit => {
            let Some(host) = host else {
                return Err(ControlCommandError::unknown(
                    "pane split requires a process host",
                    command.pane_id.clone(),
                ));
            };
            apply_pane_split_command_with_session_actor(actor, host, command)?;
            Ok(ControlCommandOutcome::Continue)
        }
        protocol::ControlCommandKind::TabNew => {
            let Some(host) = host else {
                return Err(ControlCommandError::unknown(
                    "tab new requires a process host",
                    None,
                ));
            };
            apply_tab_new_command_with_session_actor(actor, host, command)?;
            Ok(ControlCommandOutcome::Continue)
        }
        protocol::ControlCommandKind::TabClose => {
            apply_tab_close_command_with_session_actor(actor, host, command)?;
            Ok(ControlCommandOutcome::Continue)
        }
        protocol::ControlCommandKind::TabSwitch => {
            apply_tab_switch_command_with_session_actor(actor, command)?;
            Ok(ControlCommandOutcome::Continue)
        }
        protocol::ControlCommandKind::SessionNew => Err(ControlCommandError::unknown(
            "session new requires daemon registry routing",
            None,
        )),
        protocol::ControlCommandKind::SessionKill => {
            apply_session_kill_command(actor.session(), command)?;
            enqueue_lifecycle_session_event(
                actor,
                command,
                SessionEvent::RequestSessionShutdown {
                    session_id: command.session_id.clone(),
                },
            );
            Ok(ControlCommandOutcome::Shutdown)
        }
        _ => Err(ControlCommandError::unknown(
            format!("unknown control command kind {}", command.kind.0),
            command.pane_id.clone(),
        )),
    }
}

fn apply_session_kill_command(
    session: &Session,
    command: &ControlCommandSummary,
) -> Result<(), ControlCommandError> {
    if let Some(session_id) = command.session_id.as_deref()
        && session_id != session.id
    {
        return Err(ControlCommandError {
            code: protocol::ErrorCode::SessionNotFound,
            message: format!("session not found: {session_id}"),
            pane_id: None,
        });
    }
    Ok(())
}

fn enqueue_control_session_event(
    actor: &mut SessionActor,
    command: &ControlCommandSummary,
    event: SessionEvent,
) -> bool {
    actor.enqueue(PendingSessionEvent::new(
        command.actor_id.clone(),
        SessionEventLane::Control,
        command.command_seq,
        event,
    ));
    actor
        .drain_ready()
        .iter()
        .any(|record| !record.effects.is_empty())
}

fn enqueue_lifecycle_session_event(
    actor: &mut SessionActor,
    command: &ControlCommandSummary,
    event: SessionEvent,
) {
    actor.enqueue(PendingSessionEvent::new(
        command.actor_id.clone(),
        SessionEventLane::Lifecycle,
        command.command_seq,
        event,
    ));
    actor.drain_ready();
}

fn apply_pane_split_command(
    session: &mut Session,
    host: &mut dyn ProcessHost,
    command: &ControlCommandSummary,
) -> Result<(), ControlCommandError> {
    let target_pane_id = match command.pane_id.as_deref() {
        Some(pane_id) => pane_id.to_owned(),
        None => session
            .active_pane_id()
            .ok_or_else(|| ControlCommandError::pane_not_found("active pane"))?
            .to_owned(),
    };
    if !matches!(
        command.split_axis,
        protocol::SplitAxis::Horizontal | protocol::SplitAxis::Vertical
    ) {
        return Err(ControlCommandError::unknown(
            "pane split requires horizontal or vertical axis",
            Some(target_pane_id),
        ));
    }
    let template = session
        .pane_host(&target_pane_id)
        .ok_or_else(|| ControlCommandError::pane_not_found(&target_pane_id))?
        .clone();
    let new_pane_id = next_pane_id(session);
    let new_host = host_with_pane_environment(template, &session.id, &new_pane_id);
    host.start_pane(&new_pane_id, &new_host)
        .map_err(|err| ControlCommandError::unknown(err.to_string(), Some(new_pane_id.clone())))?;
    if !session.split_pane(
        &target_pane_id,
        command.split_axis,
        new_pane_id.clone(),
        new_host,
    ) {
        let _ = host.stop_pane(&new_pane_id);
        return Err(ControlCommandError::unknown(
            format!("failed to split pane: {target_pane_id}"),
            Some(target_pane_id),
        ));
    }
    Ok(())
}

fn apply_pane_split_command_with_session_actor(
    actor: &mut SessionActor,
    host: &mut dyn ProcessHost,
    command: &ControlCommandSummary,
) -> Result<(), ControlCommandError> {
    let session = actor.session();
    let target_pane_id = match command.pane_id.as_deref() {
        Some(pane_id) => pane_id.to_owned(),
        None => session
            .active_pane_id()
            .ok_or_else(|| ControlCommandError::pane_not_found("active pane"))?
            .to_owned(),
    };
    if !matches!(
        command.split_axis,
        protocol::SplitAxis::Horizontal | protocol::SplitAxis::Vertical
    ) {
        return Err(ControlCommandError::unknown(
            "pane split requires horizontal or vertical axis",
            Some(target_pane_id),
        ));
    }
    let template = session
        .pane_host(&target_pane_id)
        .ok_or_else(|| ControlCommandError::pane_not_found(&target_pane_id))?
        .clone();
    let new_pane_id = next_pane_id(session);
    let new_host = host_with_pane_environment(template, &session.id, &new_pane_id);
    host.start_pane(&new_pane_id, &new_host)
        .map_err(|err| ControlCommandError::unknown(err.to_string(), Some(new_pane_id.clone())))?;
    if !enqueue_control_session_event(
        actor,
        command,
        SessionEvent::SplitPane {
            pane_id: target_pane_id.clone(),
            axis: command.split_axis,
            new_pane_id: new_pane_id.clone(),
            new_host,
        },
    ) {
        let _ = host.stop_pane(&new_pane_id);
        return Err(ControlCommandError::unknown(
            format!("failed to split pane: {target_pane_id}"),
            Some(target_pane_id),
        ));
    }
    Ok(())
}

fn apply_tab_new_command(
    session: &mut Session,
    host: &mut dyn ProcessHost,
    command: &ControlCommandSummary,
) -> Result<(), ControlCommandError> {
    let active_pane_id = session
        .active_pane_id()
        .ok_or_else(|| ControlCommandError::pane_not_found("active pane"))?
        .to_owned();
    let template = session
        .pane_host(&active_pane_id)
        .ok_or_else(|| ControlCommandError::pane_not_found(&active_pane_id))?
        .clone();
    let tab_id = command
        .tab_id
        .clone()
        .unwrap_or_else(|| next_tab_id(session));
    if session.tabs.iter().any(|tab| tab.id == tab_id) {
        return Err(ControlCommandError::unknown(
            format!("tab already exists: {tab_id}"),
            None,
        ));
    }
    let pane_id = format!("{tab_id}-pane-1");
    let title = command.title.clone().unwrap_or_else(|| tab_id.clone());
    let new_host = host_with_pane_environment(template, &session.id, &pane_id);
    host.start_pane(&pane_id, &new_host)
        .map_err(|err| ControlCommandError::unknown(err.to_string(), Some(pane_id.clone())))?;
    if !session.add_tab(tab_id.clone(), title, pane_id.clone(), new_host)
        || (!session.switch_tab(&tab_id) && session.active_tab_id != tab_id)
    {
        let _ = host.stop_pane(&pane_id);
        return Err(ControlCommandError::unknown(
            format!("failed to create tab: {tab_id}"),
            None,
        ));
    }
    Ok(())
}

fn apply_tab_new_command_with_session_actor(
    actor: &mut SessionActor,
    host: &mut dyn ProcessHost,
    command: &ControlCommandSummary,
) -> Result<(), ControlCommandError> {
    let session = actor.session();
    let active_pane_id = session
        .active_pane_id()
        .ok_or_else(|| ControlCommandError::pane_not_found("active pane"))?
        .to_owned();
    let template = session
        .pane_host(&active_pane_id)
        .ok_or_else(|| ControlCommandError::pane_not_found(&active_pane_id))?
        .clone();
    let tab_id = command
        .tab_id
        .clone()
        .unwrap_or_else(|| next_tab_id(session));
    if session.tabs.iter().any(|tab| tab.id == tab_id) {
        return Err(ControlCommandError::unknown(
            format!("tab already exists: {tab_id}"),
            None,
        ));
    }
    let pane_id = format!("{tab_id}-pane-1");
    let title = command.title.clone().unwrap_or_else(|| tab_id.clone());
    let new_host = host_with_pane_environment(template, &session.id, &pane_id);
    host.start_pane(&pane_id, &new_host)
        .map_err(|err| ControlCommandError::unknown(err.to_string(), Some(pane_id.clone())))?;
    let added = enqueue_control_session_event(
        actor,
        command,
        SessionEvent::AddTab {
            tab_id: tab_id.clone(),
            title,
            pane_id: pane_id.clone(),
            host: new_host,
        },
    );
    let switched = enqueue_control_session_event(
        actor,
        command,
        SessionEvent::SwitchTab {
            tab_id: tab_id.clone(),
        },
    );
    if !added || (!switched && actor.session().active_tab_id != tab_id) {
        let _ = host.stop_pane(&pane_id);
        return Err(ControlCommandError::unknown(
            format!("failed to create tab: {tab_id}"),
            None,
        ));
    }
    Ok(())
}

fn apply_tab_close_command(
    session: &mut Session,
    host: Option<&mut dyn ProcessHost>,
    command: &ControlCommandSummary,
) -> Result<(), ControlCommandError> {
    let tab_id = command
        .tab_id
        .clone()
        .unwrap_or_else(|| session.active_tab_id.clone());
    let tab = session
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .ok_or_else(|| ControlCommandError::unknown(format!("tab not found: {tab_id}"), None))?;
    let pane_ids = tab_leaf_pane_ids(tab);
    if !session.close_tab(&tab_id) {
        return Err(ControlCommandError::unknown(
            format!("failed to close tab: {tab_id}"),
            None,
        ));
    }
    if let Some(host) = host {
        for pane_id in pane_ids {
            host.stop_pane(&pane_id)
                .map_err(|err| ControlCommandError::unknown(err.to_string(), Some(pane_id)))?;
        }
    }
    Ok(())
}

fn apply_tab_close_command_with_session_actor(
    actor: &mut SessionActor,
    host: Option<&mut dyn ProcessHost>,
    command: &ControlCommandSummary,
) -> Result<(), ControlCommandError> {
    let tab_id = command
        .tab_id
        .clone()
        .unwrap_or_else(|| actor.session().active_tab_id.clone());
    let tab = actor
        .session()
        .tabs
        .iter()
        .find(|tab| tab.id == tab_id)
        .ok_or_else(|| ControlCommandError::unknown(format!("tab not found: {tab_id}"), None))?;
    let pane_ids = tab_leaf_pane_ids(tab);
    if !enqueue_control_session_event(
        actor,
        command,
        SessionEvent::CloseTab {
            tab_id: tab_id.clone(),
        },
    ) {
        return Err(ControlCommandError::unknown(
            format!("failed to close tab: {tab_id}"),
            None,
        ));
    }
    if let Some(host) = host {
        for pane_id in pane_ids {
            host.stop_pane(&pane_id)
                .map_err(|err| ControlCommandError::unknown(err.to_string(), Some(pane_id)))?;
        }
    }
    Ok(())
}

fn apply_tab_switch_command(
    session: &mut Session,
    command: &ControlCommandSummary,
) -> Result<(), ControlCommandError> {
    let tab_id = required_control_tab_id(command)?;
    if !session.tabs.iter().any(|tab| tab.id == tab_id) {
        return Err(ControlCommandError::unknown(
            format!("tab not found: {tab_id}"),
            None,
        ));
    }
    if !session.switch_tab(&tab_id) && session.active_tab_id != tab_id {
        return Err(ControlCommandError::unknown(
            format!("failed to switch tab: {tab_id}"),
            None,
        ));
    }
    Ok(())
}

fn apply_tab_switch_command_with_session_actor(
    actor: &mut SessionActor,
    command: &ControlCommandSummary,
) -> Result<(), ControlCommandError> {
    let tab_id = required_control_tab_id(command)?;
    if !actor.session().tabs.iter().any(|tab| tab.id == tab_id) {
        return Err(ControlCommandError::unknown(
            format!("tab not found: {tab_id}"),
            None,
        ));
    }
    if !enqueue_control_session_event(
        actor,
        command,
        SessionEvent::SwitchTab {
            tab_id: tab_id.clone(),
        },
    ) && actor.session().active_tab_id != tab_id
    {
        return Err(ControlCommandError::unknown(
            format!("failed to switch tab: {tab_id}"),
            None,
        ));
    }
    Ok(())
}

fn required_control_tab_id(command: &ControlCommandSummary) -> Result<String, ControlCommandError> {
    command
        .tab_id
        .clone()
        .filter(|tab_id| !tab_id.is_empty())
        .ok_or_else(|| ControlCommandError::unknown("tab switch requires a tab ID", None))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ControlCommandError {
    code: protocol::ErrorCode,
    message: String,
    pane_id: Option<String>,
}

impl ControlCommandError {
    fn pane_not_found(pane_id: &str) -> Self {
        Self {
            code: protocol::ErrorCode::PaneNotFound,
            message: format!("pane not found: {pane_id}"),
            pane_id: Some(pane_id.to_owned()),
        }
    }

    fn unknown(message: impl Into<String>, pane_id: Option<String>) -> Self {
        Self {
            code: protocol::ErrorCode::Unknown,
            message: message.into(),
            pane_id,
        }
    }
}

fn next_pane_id(session: &Session) -> String {
    let mut next = 1;
    loop {
        let pane_id = format!("pane-{next}");
        if session.pane_host(&pane_id).is_none() {
            return pane_id;
        }
        next += 1;
    }
}

fn next_tab_id(session: &Session) -> String {
    let mut next = 1;
    loop {
        let tab_id = format!("tab-{next}");
        if !session.tabs.iter().any(|tab| tab.id == tab_id) {
            return tab_id;
        }
        next += 1;
    }
}

fn host_with_pane_environment(mut host: HostSpec, session_id: &str, pane_id: &str) -> HostSpec {
    let socket = host
        .command
        .env
        .iter()
        .find(|(key, _)| key == "NMUX_SOCKET")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    let previous_origin = host
        .command
        .env
        .iter()
        .find(|(key, _)| key == "NMUX_ORIGIN")
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| host.id.clone());
    host.command.env.retain(|(key, _)| {
        !matches!(
            key.as_str(),
            "NMUX" | "NMUX_SESSION_ID" | "NMUX_PANE_ID" | "NMUX_SOCKET" | "NMUX_ORIGIN"
        )
    });
    host.command.env.extend([
        ("NMUX".to_owned(), "1".to_owned()),
        ("NMUX_SESSION_ID".to_owned(), session_id.to_owned()),
        ("NMUX_PANE_ID".to_owned(), pane_id.to_owned()),
        ("NMUX_SOCKET".to_owned(), socket),
        ("NMUX_ORIGIN".to_owned(), previous_origin),
    ]);
    host
}

fn tab_leaf_pane_ids(tab: &nmux_core::session::Tab) -> Vec<String> {
    let mut pane_ids = Vec::new();
    collect_tab_leaf_pane_ids(&tab.root, &mut pane_ids);
    pane_ids
}

fn collect_tab_leaf_pane_ids(pane: &nmux_core::session::Pane, pane_ids: &mut Vec<String>) {
    if pane.children.is_empty() {
        pane_ids.push(pane.id.clone());
        return;
    }
    for child in &pane.children {
        collect_tab_leaf_pane_ids(child, pane_ids);
    }
}
