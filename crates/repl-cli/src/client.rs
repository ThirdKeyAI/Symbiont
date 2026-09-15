use anyhow::{bail, Result};
use repl_proto::{ErrorObject, EvaluateParams, EvaluateResult, Request};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

pub struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    request_id: u64,
}

impl Client {
    pub fn new() -> Result<Self> {
        let mut cmd = Command::new(std::env::current_exe()?);
        cmd.arg("--stdio");
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());

        Ok(Self {
            child,
            stdin,
            stdout,
            request_id: 0,
        })
    }

    pub fn evaluate(&mut self, code: &str) -> Result<String> {
        self.request_id += 1;
        let params = EvaluateParams {
            code: code.to_string(),
        };
        let request = Request {
            id: self.request_id,
            method: "evaluate".to_string(),
            params: serde_json::to_value(params)?,
        };

        let request_json = serde_json::to_string(&request)? + "\n";
        self.stdin.write_all(request_json.as_bytes())?;

        let mut line = String::new();
        self.stdout.read_line(&mut line)?;
        decode_response(&line, self.request_id)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

fn decode_response(line: &str, expected_id: u64) -> Result<String> {
    #[derive(serde::Deserialize)]
    struct Reply {
        id: u64,
        result: Option<EvaluateResult>,
        error: Option<ErrorObject>,
    }
    let reply: Reply = serde_json::from_str(line)?;
    if reply.id != expected_id {
        bail!("REPL response does not match request {expected_id}");
    }
    match (reply.result, reply.error) {
        (Some(result), None) => Ok(result.output),
        (None, Some(error)) => bail!("{}", error.message),
        _ => bail!("REPL response must contain exactly one result or error"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_responses_preserve_outputs_and_execution_errors() {
        assert_eq!(
            decode_response(r#"{"id":7,"result":{"output":"done"}}"#, 7).unwrap(),
            "done"
        );
        let error = decode_response(
            r#"{"id":7,"error":{"code":-32000,"message":"Policy denied"}}"#,
            7,
        )
        .unwrap_err();
        assert_eq!(error.to_string(), "Policy denied");
        for response in [
            r#"{"id":8,"result":{"output":"wrong request"}}"#,
            r#"{"id":7}"#,
            r#"{"id":7,"result":{"output":"done"},"error":{"code":1,"message":"failed"}}"#,
            "",
        ] {
            assert!(decode_response(response, 7).is_err());
        }
    }
}
