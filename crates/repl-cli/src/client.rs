use anyhow::Result;
use repl_proto::{EvaluateParams, Request, Response};
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
        let response: Response = serde_json::from_str(&line)?;

        let result: String = serde_json::from_value(response.result)?;
        Ok(result)
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.child.kill().ok();
    }
}
