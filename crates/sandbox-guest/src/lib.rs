//! Bounded host/guest framing. Headers are length-prefixed JSON; stream bytes
//! follow separately, so binary input/output never becomes command syntax.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
};

pub mod files;
pub mod snapshot;
pub mod stream;

pub const VERSION: u32 = 5;
pub const PORT: u32 = 4050;
pub const MAX_HEADER: usize = 1024 * 1024;
pub const MAX_INPUT: usize = 10 * 1024 * 1024;
pub const MAX_OUTPUT: usize = 10 * 1024 * 1024;
pub const INPUT_FILE: &str = "/tmp/symbi-parser-input";
pub const MAX_PROCESSES: u64 = 256;
pub const MAX_FILES: u64 = 256;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hello {
    pub version: u32,
    pub implementation: String,
}
pub const IMPLEMENTATION: &str = env!("SYMBI_GUEST_IMPLEMENTATION");

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommandMode {
    OneShot,
    Stdio,
    Pty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub version: u32,
    pub id: String,
    pub mode: CommandMode,
    pub argv: Vec<String>,
    pub environment: BTreeMap<String, String>,
    pub working_dir: String,
    pub input_length: usize,
    pub input_as_file: bool,
    pub files: Option<files::Grant>,
    pub snapshot: Option<snapshot::Grant>,
    pub max_output_bytes: usize,
    pub timeout_ms: u64,
}
impl Command {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.version != VERSION
            || self.id.len() != 36
            || !self.id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
        {
            anyhow::bail!("invalid guest command protocol or identity");
        }
        if self.argv.is_empty()
            || self.argv.len() > 1024
            || self.argv[0].is_empty()
            || self.argv.iter().any(|v| v.contains('\0'))
            || self.argv.iter().map(String::len).sum::<usize>() > 512 * 1024
        {
            anyhow::bail!("invalid guest argv");
        }
        if !self.working_dir.starts_with('/')
            || self.working_dir.contains('\0')
            || self.working_dir.len() > 4096
        {
            anyhow::bail!("guest working directory must be an absolute path");
        }
        let mut environment_bytes = 0usize;
        for (name, value) in &self.environment {
            if name.is_empty() || name.contains(['=', '\0']) || value.contains('\0') {
                anyhow::bail!("invalid guest environment");
            }
            environment_bytes = environment_bytes
                .saturating_add(name.len())
                .saturating_add(value.len())
                .saturating_add(2);
        }
        if environment_bytes > 65536
            || self.input_length > MAX_INPUT
            || (self.mode != CommandMode::OneShot && (self.input_length != 0 || self.input_as_file))
            || !(1..=MAX_OUTPUT).contains(&self.max_output_bytes)
            || !(1..=86_400_000).contains(&self.timeout_ms)
        {
            anyhow::bail!("guest command limits are invalid");
        }
        if let Some(snapshot) = &self.snapshot {
            snapshot.validate()?;
            anyhow::ensure!(
                self.mode == CommandMode::OneShot
                    && self.files.is_none()
                    && !self.input_as_file
                    && self.input_length == 0,
                "snapshot transfer requires an exclusive oneshot command"
            );
        }
        if let Some(files) = &self.files {
            files.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Outcome {
    pub version: u32,
    pub id: String,
    pub exit_code: i32,
    pub stdout_length: usize,
    pub stderr_length: usize,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
    pub timed_out: bool,
    pub error: Option<String>,
    pub file_output: Option<files::Receipt>,
    pub snapshot: Option<snapshot::Receipt>,
    pub finalized: bool,
}
impl Outcome {
    pub fn validate(&self, request: &Command) -> anyhow::Result<()> {
        if self.version != VERSION
            || self.id != request.id
            || self.stdout_length > request.max_output_bytes
            || self.stderr_length > request.max_output_bytes
            || !(-128..=255).contains(&self.exit_code)
            || self.error.as_ref().is_some_and(|e| e.len() > 1024)
            || (self.finalized
                && (request.mode == CommandMode::OneShot
                    || request
                        .files
                        .as_ref()
                        .is_none_or(|grant| grant.output.is_none())))
        {
            anyhow::bail!("invalid or mismatched guest outcome");
        }
        match (&self.snapshot, &request.snapshot) {
            (Some(receipt), Some(grant)) => receipt.validate(grant)?,
            (None, None) => (),
            _ => anyhow::bail!("missing or unsolicited guest snapshot receipt"),
        }
        match (&self.file_output, &request.files) {
            (Some(receipt), Some(grant)) if self.files_succeeded() => receipt.validate(grant)?,
            (None, grant)
                if !self.files_succeeded() || grant.as_ref().is_none_or(|g| g.output.is_none()) => {
            }
            _ => anyhow::bail!("missing or unsolicited guest file output"),
        }
        Ok(())
    }
    pub fn success(&self) -> bool {
        !self.finalized
            && self.exit_code == 0
            && !self.timed_out
            && !self.stdout_truncated
            && !self.stderr_truncated
            && self.error.is_none()
    }
    /// An explicit streaming finalization may stop a persistent process with
    /// SIGKILL. Preserve that observed status; do not invent a natural exit.
    pub fn files_succeeded(&self) -> bool {
        self.success()
            || (self.finalized
                && matches!(self.exit_code, 0 | -9)
                && !self.timed_out
                && !self.stdout_truncated
                && !self.stderr_truncated
                && self.error.is_none())
    }
}

pub fn encode_header(value: &impl Serialize) -> anyhow::Result<Vec<u8>> {
    let data = serde_json::to_vec(value)?;
    if data.is_empty() || data.len() > MAX_HEADER {
        anyhow::bail!("guest header exceeds limit");
    }
    Ok(data)
}
pub fn write_header(writer: &mut impl Write, value: &impl Serialize) -> anyhow::Result<()> {
    let data = encode_header(value)?;
    writer.write_all(&(data.len() as u32).to_be_bytes())?;
    writer.write_all(&data)?;
    Ok(())
}
pub fn read_header<T: serde::de::DeserializeOwned>(reader: &mut impl Read) -> anyhow::Result<T> {
    let mut length = [0; 4];
    reader.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_HEADER {
        anyhow::bail!("invalid guest header length");
    }
    let mut data = vec![0; length];
    reader.read_exact(&mut data)?;
    Ok(serde_json::from_slice(&data)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framing_refuses_oversize_truncation_and_unknown_fields() {
        assert!(read_header::<Hello>(&mut &u32::MAX.to_be_bytes()[..]).is_err());
        assert!(read_header::<Hello>(&mut &[0, 0, 0, 10, b'{'][..]).is_err());
        let mut bytes = Vec::new();
        write_header(
            &mut bytes,
            &serde_json::json!({"version":1,"implementation":"x","extra":true}),
        )
        .unwrap();
        assert!(read_header::<Hello>(&mut bytes.as_slice()).is_err());
    }
    #[test]
    fn command_round_trip_retains_exact_data() {
        let request = Command {
            version: VERSION,
            id: "01234567-89ab-cdef-0123-456789abcdef".into(),
            mode: CommandMode::OneShot,
            argv: vec!["/bin/tool".into(), " \n$(data)\"🌍".into()],
            environment: BTreeMap::from([("LABEL".into(), " line\nnext=".into())]),
            working_dir: "/tmp".into(),
            input_length: 3,
            input_as_file: false,
            files: None,
            snapshot: None,
            max_output_bytes: 1024,
            timeout_ms: 1000,
        };
        request.validate().unwrap();
        let mut bytes = Vec::new();
        write_header(&mut bytes, &request).unwrap();
        bytes.extend_from_slice(b"a\0b");
        let mut input = bytes.as_slice();
        let actual: Command = read_header(&mut input).unwrap();
        assert_eq!(actual.argv, request.argv);
        assert_eq!(actual.environment, request.environment);
        assert_eq!(input, b"a\0b");
        for mode in [CommandMode::Stdio, CommandMode::Pty] {
            let mut stream = request.clone();
            stream.mode = mode;
            assert!(stream.validate().is_err());
            stream.input_length = 0;
            stream.validate().unwrap();
            stream.input_as_file = true;
            assert!(stream.validate().is_err());
        }
        let mut bad = request.clone();
        bad.environment.insert("bad=name".into(), "x".into());
        assert!(bad.validate().is_err());
        let mut result = Outcome {
            version: VERSION,
            id: request.id.clone(),
            exit_code: 0,
            stdout_length: 0,
            stderr_length: 0,
            stdout_truncated: false,
            stderr_truncated: false,
            timed_out: false,
            error: None,
            file_output: None,
            snapshot: None,
            finalized: false,
        };
        result.validate(&request).unwrap();
        assert!(result.success());
        result.id = "another".into();
        assert!(result.validate(&request).is_err());
        result.id = request.id.clone();
        result.stdout_length = 1025;
        assert!(result.validate(&request).is_err());
        result.stdout_length = 0;
        result.timed_out = true;
        assert!(!result.success());
    }
    #[test]
    fn output_receipts_require_success_and_explicit_stream_finalization() {
        let mut request = Command {
            version: VERSION,
            id: "01234567-89ab-cdef-0123-456789abcdef".into(),
            mode: CommandMode::Stdio,
            argv: vec!["/bin/server".into()],
            environment: Default::default(),
            working_dir: "/tmp".into(),
            input_length: 0,
            input_as_file: false,
            snapshot: None,
            files: Some(files::Grant {
                inputs: vec![],
                output: Some("/tmp/result".into()),
                max_file_bytes: 1024,
            }),
            max_output_bytes: 1024,
            timeout_ms: 1000,
        };
        let mut result = Outcome {
            version: VERSION,
            id: request.id.clone(),
            exit_code: -9,
            stdout_length: 0,
            stderr_length: 0,
            stdout_truncated: false,
            stderr_truncated: false,
            timed_out: false,
            error: None,
            finalized: true,
            snapshot: None,
            file_output: Some(files::Receipt {
                path: "/tmp/result".into(),
                length: 1,
                sha256: "a".repeat(64),
            }),
        };
        result.validate(&request).unwrap();
        assert!(!result.success() && result.files_succeeded());
        for code in [17, -15, 125] {
            result.exit_code = code;
            assert!(!result.files_succeeded());
            assert!(result.validate(&request).is_err());
        }
        result.exit_code = -9;
        result.finalized = false;
        assert!(result.validate(&request).is_err());
        result.finalized = true;
        request.mode = CommandMode::OneShot;
        assert!(result.validate(&request).is_err());
        result.finalized = false;
        result.exit_code = 0;
        result.validate(&request).unwrap();
        assert!(result.success());
        result.file_output = None;
        assert!(result.validate(&request).is_err());
        result.exit_code = 17;
        result.validate(&request).unwrap();
    }
}
