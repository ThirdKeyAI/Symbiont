use anyhow::Result;
use chrono::Utc;
use repl_core::Session;
use repl_proto::{CommandLog, OutputLog};
use std::fs::{File, OpenOptions};
use std::io::Write;

pub struct SessionManager {
    session: Session,
    recorder: Option<File>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            session: Session::new(),
            recorder: None,
        }
    }

    pub fn snapshot(&self, path: &str) -> Result<()> {
        let data = self.session.snapshot()?;
        std::fs::write(path, data)?;
        Ok(())
    }

    pub fn restore(&mut self, path: &str) -> Result<()> {
        let data = std::fs::read_to_string(path)?;
        self.session = Session::restore(&data)?;
        Ok(())
    }

    pub fn start_recording(&mut self, path: &str) -> Result<()> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        self.recorder = Some(file);
        Ok(())
    }

    pub fn stop_recording(&mut self) {
        self.recorder = None;
    }

    pub fn record_command(&mut self, command: &str) {
        if let Some(recorder) = &mut self.recorder {
            let log = CommandLog {
                timestamp: Utc::now().to_rfc3339(),
                command: command.to_string(),
            };
            if let Ok(json) = serde_json::to_string(&log) {
                writeln!(recorder, "{}", json).ok();
            }
        }
    }

    pub fn record_output(&mut self, output: &str) {
        if let Some(recorder) = &mut self.recorder {
            let log = OutputLog {
                timestamp: Utc::now().to_rfc3339(),
                output: output.to_string(),
            };
            if let Ok(json) = serde_json::to_string(&log) {
                writeln!(recorder, "{}", json).ok();
            }
        }
    }
}
