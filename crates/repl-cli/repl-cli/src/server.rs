use anyhow::Result;
use repl_core::{evaluate, parse_policy};
use repl_proto::{ErrorObject, EvaluateParams, Request, Response};
use std::io::{self, BufRead};

pub fn run() -> Result<()> {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let request: Request = serde_json::from_str(&line)?;

        let response = match request.method.as_str() {
            "evaluate" => {
                let params: EvaluateParams = serde_json::from_value(request.params)?;
                let result = evaluate(&params.code);
                match result {
                    Ok(output) => Response {
                        id: request.id,
                        result: serde_json::to_value(output)?,
                    },
                    Err(e) => {
                        // This is a placeholder for a proper error response
                        let error = ErrorObject {
                            code: -32000,
                            message: e.to_string(),
                            data: None,
                        };
                        // This part is not fully implemented yet, just logging
                        eprintln!("Error: {:?}", error);
                        continue;
                    }
                }
            }
            "validate_policy" => {
                // Similar to evaluate, but for policy
                // Placeholder implementation
                let params: serde_json::Value = serde_json::from_value(request.params)?;
                let policy_str = params.get("policy").and_then(|v| v.as_str()).unwrap_or_default();
                let result = parse_policy(policy_str);
                 match result {
                    Ok(_) => Response {
                        id: request.id,
                        result: serde_json::to_value("Policy is valid")?,
                    },
                    Err(e) => {
                        let error = ErrorObject {
                            code: -32001,
                            message: e.to_string(),
                            data: None,
                        };
                        eprintln!("Error: {:?}", error);
                        continue;
                    }
                }
            }
            _ => {
                let error = ErrorObject {
                    code: -32601,
                    message: "Method not found".to_string(),
                    data: None,
                };
                eprintln!("Error: {:?}", error);
                continue;
            }
        };

        let response_json = serde_json::to_string(&response)?;
        println!("{}", response_json);
    }
    Ok(())
}