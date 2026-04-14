use super::ManagerWorkerDaemonArgs;
use super::runtime::ManagerRuntime;
use super::worker_pool_process::WorkerProcess;
use codex_specialist::SpecialistWorkerCommand;
use std::collections::BTreeMap;
use std::io::BufRead;
use std::io::IsTerminal;
use std::io::Write;
use std::path::Path;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Instant;

pub(super) fn start_console_if_enabled<W: Write>(
    args: &ManagerWorkerDaemonArgs,
    writer: &mut W,
) -> anyhow::Result<Option<Receiver<ConsoleMessage>>> {
    let enabled = args.interactive || (!args.no_interactive && std::io::stdin().is_terminal());
    if !enabled {
        return Ok(None);
    }

    let (sender, receiver) = mpsc::channel();
    spawn_console_reader(sender);
    writeln!(
        writer,
        "interactive console enabled; type `help`, `agents`, `chat <agent>`, `@<agent> <message>`, or `quit`"
    )?;
    Ok(Some(receiver))
}

#[derive(Default)]
pub(super) struct ConsoleState {
    chat_agent_id: Option<String>,
}

impl ManagerRuntime {
    pub(super) fn drain_console_messages<W: Write>(
        &self,
        receiver: &Receiver<ConsoleMessage>,
        state: &mut ConsoleState,
        prompt_queue_dir: Option<&Path>,
        workers: &mut BTreeMap<String, WorkerProcess>,
        writer: &mut W,
    ) -> anyhow::Result<bool> {
        let mut shutdown_requested = false;
        loop {
            match receiver.try_recv() {
                Ok(ConsoleMessage::Command(command)) => {
                    shutdown_requested |= self.handle_console_command(
                        command,
                        state,
                        prompt_queue_dir,
                        workers,
                        writer,
                    )?;
                }
                Ok(ConsoleMessage::ParseError(err)) => {
                    writeln!(writer, "console error: {err}")?;
                    print_console_help(writer)?;
                }
                Ok(ConsoleMessage::Closed) => {
                    writeln!(writer, "console input closed")?;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }
        Ok(shutdown_requested)
    }

    fn handle_console_command<W: Write>(
        &self,
        command: ConsoleCommand,
        state: &mut ConsoleState,
        prompt_queue_dir: Option<&Path>,
        workers: &mut BTreeMap<String, WorkerProcess>,
        writer: &mut W,
    ) -> anyhow::Result<bool> {
        match command {
            ConsoleCommand::Help => print_console_help(writer)?,
            ConsoleCommand::Agents => {
                for worker in workers.values() {
                    let state = if worker.busy { "busy" } else { "idle" };
                    writeln!(writer, "{}\t{state}", worker.agent.agent_id)?;
                }
            }
            ConsoleCommand::Status { agent_id } => {
                if let Some(agent_id) = agent_id {
                    let Some(worker) = workers.get_mut(&agent_id) else {
                        writeln!(writer, "unknown worker agent_id={agent_id}")?;
                        return Ok(false);
                    };
                    self.send_status_request(worker, "console", writer)?;
                } else {
                    for worker in workers.values_mut() {
                        self.send_status_request(worker, "console", writer)?;
                    }
                }
            }
            ConsoleCommand::Prompt { agent_id, prompt } => {
                self.send_prompt_to_worker_or_queue(
                    &agent_id,
                    &prompt,
                    prompt_queue_dir,
                    workers,
                    writer,
                )?;
            }
            ConsoleCommand::ChatTarget { agent_id } => {
                if let Some(agent_id) = agent_id {
                    if !workers.contains_key(&agent_id) {
                        writeln!(writer, "unknown worker agent_id={agent_id}")?;
                        return Ok(false);
                    }
                    state.chat_agent_id = Some(agent_id.clone());
                    writeln!(
                        writer,
                        "chat target set agent_id={agent_id}; plain input will prompt this worker"
                    )?;
                } else {
                    state.chat_agent_id = None;
                    writeln!(writer, "chat target cleared")?;
                }
            }
            ConsoleCommand::ChatMessage { prompt } => {
                let Some(agent_id) = state.chat_agent_id.clone() else {
                    writeln!(
                        writer,
                        "no chat target set; use `chat <agent-id>` or `@<agent-id> <message>`"
                    )?;
                    return Ok(false);
                };
                self.send_prompt_to_worker_or_queue(
                    &agent_id,
                    &prompt,
                    prompt_queue_dir,
                    workers,
                    writer,
                )?;
            }
            ConsoleCommand::Quit => {
                writeln!(writer, "worker-daemon shutdown requested from console")?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn send_prompt_to_worker_or_queue<W: Write>(
        &self,
        agent_id: &str,
        prompt: &str,
        prompt_queue_dir: Option<&Path>,
        workers: &mut BTreeMap<String, WorkerProcess>,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let Some(worker) = workers.get(agent_id) else {
            writeln!(writer, "unknown worker agent_id={agent_id}")?;
            return Ok(());
        };
        if worker.busy {
            if let Some(prompt_queue_dir) = prompt_queue_dir {
                writeln!(writer, "worker busy agent_id={agent_id}; prompt queued")?;
                self.worker_enqueue(agent_id, prompt, Some(prompt_queue_dir), writer)?;
            } else {
                writeln!(
                    writer,
                    "worker busy agent_id={agent_id}; prompt not sent because prompt queue is disabled"
                )?;
            }
            return Ok(());
        }

        let Some(worker) = workers.get_mut(agent_id) else {
            writeln!(writer, "unknown worker agent_id={agent_id}")?;
            return Ok(());
        };
        let id = worker.next_command_id("prompt");
        worker.send(SpecialistWorkerCommand::Prompt {
            id,
            prompt: prompt.to_string(),
        })?;
        worker.busy = true;
        self.log_event(format!("sent interactive worker prompt to {agent_id}"))?;
        writeln!(writer, "worker prompt sent agent_id={agent_id}")?;
        Ok(())
    }

    fn send_status_request<W: Write>(
        &self,
        worker: &mut WorkerProcess,
        reason: &str,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        if worker.busy {
            writeln!(
                writer,
                "worker busy agent_id={} status_request=skipped reason={reason}",
                worker.agent.agent_id
            )?;
            return Ok(());
        }

        let id = worker.next_command_id("status");
        match worker.send(SpecialistWorkerCommand::Status { id }) {
            Ok(()) => {
                worker.last_status_request = Some(Instant::now());
                writeln!(
                    writer,
                    "worker status requested agent_id={} reason={reason}",
                    worker.agent.agent_id
                )?;
            }
            Err(err) => writeln!(
                writer,
                "worker status request failed agent_id={} reason={reason}: {err:#}",
                worker.agent.agent_id
            )?,
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ConsoleCommand {
    Help,
    Agents,
    Status { agent_id: Option<String> },
    Prompt { agent_id: String, prompt: String },
    ChatTarget { agent_id: Option<String> },
    ChatMessage { prompt: String },
    Quit,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ConsoleMessage {
    Command(ConsoleCommand),
    ParseError(String),
    Closed,
}

fn spawn_console_reader(sender: Sender<ConsoleMessage>) {
    thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else {
                break;
            };
            let message = match parse_console_command(&line) {
                Ok(Some(command)) => ConsoleMessage::Command(command),
                Ok(None) => continue,
                Err(err) => ConsoleMessage::ParseError(err),
            };
            if sender.send(message).is_err() {
                return;
            }
        }
        let _ = sender.send(ConsoleMessage::Closed);
    });
}

fn print_console_help<W: Write>(writer: &mut W) -> anyhow::Result<()> {
    writeln!(writer, "console commands:")?;
    writeln!(writer, "  help")?;
    writeln!(writer, "  agents")?;
    writeln!(writer, "  status [agent-id|all]")?;
    writeln!(writer, "  prompt <agent-id> <message>")?;
    writeln!(writer, "  chat <agent-id>")?;
    writeln!(writer, "  chat off")?;
    writeln!(writer, "  @<agent-id> <message>")?;
    writeln!(writer, "  plain text after chat <agent-id>")?;
    writeln!(writer, "  quit")?;
    Ok(())
}

fn parse_console_command(line: &str) -> Result<Option<ConsoleCommand>, String> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(None);
    }
    if let Some(prompt) = line.strip_prefix('@') {
        return parse_targeted_prompt(prompt);
    }

    let forced_command = line.starts_with('/');
    let line = line.strip_prefix('/').unwrap_or(line);
    let mut parts = line.splitn(3, char::is_whitespace);
    let command = parts.next().unwrap_or_default();
    let first_arg = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let remaining = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    match command {
        "help" | "?" if first_arg.is_none() => Ok(Some(ConsoleCommand::Help)),
        "agents" | "list" if first_arg.is_none() => Ok(Some(ConsoleCommand::Agents)),
        "quit" | "exit" if first_arg.is_none() => Ok(Some(ConsoleCommand::Quit)),
        "status" => {
            if remaining.is_some() {
                return parse_command_or_chat_message(
                    forced_command,
                    "usage: status [agent-id|all]",
                    line,
                );
            }
            let agent_id = first_arg
                .filter(|value| *value != "all")
                .map(ToString::to_string);
            Ok(Some(ConsoleCommand::Status { agent_id }))
        }
        "prompt" => {
            let agent_id =
                first_arg.ok_or_else(|| "usage: prompt <agent-id> <message>".to_string())?;
            let prompt =
                remaining.ok_or_else(|| "usage: prompt <agent-id> <message>".to_string())?;
            Ok(Some(ConsoleCommand::Prompt {
                agent_id: agent_id.to_string(),
                prompt: prompt.to_string(),
            }))
        }
        "chat" => {
            if remaining.is_some() {
                return parse_command_or_chat_message(
                    forced_command,
                    "usage: chat <agent-id>|off",
                    line,
                );
            }
            let Some(agent_id) = first_arg else {
                return Err("usage: chat <agent-id>|off".to_string());
            };
            let agent_id = if matches!(agent_id, "off" | "none" | "clear") {
                None
            } else {
                Some(agent_id.to_string())
            };
            Ok(Some(ConsoleCommand::ChatTarget { agent_id }))
        }
        _ => parse_command_or_chat_message(
            forced_command,
            "unknown console command; try `help`, `agents`, `status [agent]`, `prompt <agent> <message>`, `chat <agent>`, `@<agent> <message>`, or `quit`",
            line,
        ),
    }
}

fn parse_targeted_prompt(line: &str) -> Result<Option<ConsoleCommand>, String> {
    let mut parts = line.trim().splitn(2, char::is_whitespace);
    let agent_id = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "usage: @<agent-id> <message>".to_string())?;
    let prompt = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "usage: @<agent-id> <message>".to_string())?;
    Ok(Some(ConsoleCommand::Prompt {
        agent_id: agent_id.to_string(),
        prompt: prompt.to_string(),
    }))
}

fn parse_command_or_chat_message(
    forced_command: bool,
    message: &str,
    line: &str,
) -> Result<Option<ConsoleCommand>, String> {
    if forced_command {
        Err(message.to_string())
    } else {
        Ok(Some(ConsoleCommand::ChatMessage {
            prompt: line.to_string(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::ConsoleCommand;
    use super::parse_console_command;
    use pretty_assertions::assert_eq;

    #[test]
    fn parses_console_prompt_command() {
        assert_eq!(
            parse_console_command("prompt agent-003 Continue the review").unwrap(),
            Some(ConsoleCommand::Prompt {
                agent_id: "agent-003".to_string(),
                prompt: "Continue the review".to_string(),
            })
        );
    }

    #[test]
    fn parses_status_all_as_no_agent_filter() {
        assert_eq!(
            parse_console_command("status all").unwrap(),
            Some(ConsoleCommand::Status { agent_id: None })
        );
    }

    #[test]
    fn parses_chat_target_command() {
        assert_eq!(
            parse_console_command("chat agent-003").unwrap(),
            Some(ConsoleCommand::ChatTarget {
                agent_id: Some("agent-003".to_string()),
            })
        );
    }

    #[test]
    fn parses_at_prompt_command() {
        assert_eq!(
            parse_console_command("@agent-003 Continue by shorthand").unwrap(),
            Some(ConsoleCommand::Prompt {
                agent_id: "agent-003".to_string(),
                prompt: "Continue by shorthand".to_string(),
            })
        );
    }

    #[test]
    fn parses_plain_text_as_chat_message() {
        assert_eq!(
            parse_console_command("Continue the next step").unwrap(),
            Some(ConsoleCommand::ChatMessage {
                prompt: "Continue the next step".to_string(),
            })
        );
    }
}
