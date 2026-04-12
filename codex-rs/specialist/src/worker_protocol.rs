use crate::SpecialistCheckpoint;
use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SpecialistWorkerCommand {
    Prompt { id: String, prompt: String },
    Status { id: String },
    Shutdown { id: String },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SpecialistWorkerEvent {
    Ready {
        workspace_id: String,
        issue_id: String,
        worker_pid: u32,
    },
    Status {
        command_id: String,
        workspace_id: String,
        issue_id: String,
        checkpoint: Option<Box<SpecialistCheckpoint>>,
    },
    TurnStarted {
        command_id: String,
    },
    TurnCompleted {
        command_id: String,
        exit_code: Option<i32>,
        final_message: Option<String>,
    },
    NeedsDirection {
        command_id: String,
        reason: String,
    },
    Error {
        command_id: Option<String>,
        message: String,
    },
    Exited {
        command_id: String,
        reason: String,
    },
}

pub fn decode_worker_command(line: &str) -> serde_json::Result<SpecialistWorkerCommand> {
    serde_json::from_str(line)
}

pub fn encode_worker_event(event: &SpecialistWorkerEvent) -> serde_json::Result<String> {
    serde_json::to_string(event)
}

#[cfg(test)]
mod tests {
    use super::SpecialistWorkerCommand;
    use super::SpecialistWorkerEvent;
    use super::decode_worker_command;
    use super::encode_worker_event;
    use pretty_assertions::assert_eq;

    #[test]
    fn prompt_command_round_trips_from_jsonl() {
        let command = decode_worker_command(
            r#"{"type":"prompt","id":"turn-1","prompt":"Continue the source hunt."}"#,
        )
        .expect("decode command");

        assert_eq!(
            command,
            SpecialistWorkerCommand::Prompt {
                id: "turn-1".to_string(),
                prompt: "Continue the source hunt.".to_string(),
            }
        );
    }

    #[test]
    fn ready_event_serializes_as_tagged_json() {
        let event = SpecialistWorkerEvent::Ready {
            workspace_id: "workspace".to_string(),
            issue_id: "issue".to_string(),
            worker_pid: 42,
        };

        assert_eq!(
            encode_worker_event(&event).expect("encode event"),
            r#"{"type":"ready","workspace_id":"workspace","issue_id":"issue","worker_pid":42}"#
        );
    }
}
