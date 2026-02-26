use anyhow::Result;
use repl_core::{ReplEngine, RuntimeBridge};
use repl_proto::{ErrorObject, ErrorResponse, EvaluateParams, EvaluateResult, Request, Response};
use serde::Deserialize;
use std::io::{self, BufRead};
use std::sync::Arc;
use symbi_runtime::reasoning::inference::InferenceProvider;
use tokio::runtime::Runtime;

#[derive(Deserialize)]
struct ExecuteParams {
    command: String,
}

pub fn run() -> Result<()> {
    let rt = Runtime::new()?;
    let runtime_bridge = Arc::new(RuntimeBridge::new());

    // Auto-detect inference provider from environment variables
    if let Some(provider) =
        symbi_runtime::reasoning::providers::cloud::CloudInferenceProvider::from_env()
    {
        eprintln!(
            "Inference provider: {} ({})",
            provider.provider_name(),
            provider.default_model()
        );
        runtime_bridge.set_inference_provider(Arc::new(provider));
    }

    let engine = ReplEngine::new(runtime_bridge);
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        let request: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let error = ErrorObject {
                    code: -32700,
                    message: "Parse error".to_string(),
                    data: Some(serde_json::Value::String(e.to_string())),
                };
                let err_resp = ErrorResponse { id: 0, error };
                println!("{}", serde_json::to_string(&err_resp)?);
                continue;
            }
        };
        let response_json = match request.method.as_str() {
            "evaluate" => match serde_json::from_value::<EvaluateParams>(request.params) {
                Ok(params) => match rt.block_on(engine.evaluate(&params.code)) {
                    Ok(output) => {
                        let result = EvaluateResult { output };
                        serde_json::to_string(&Response {
                            id: request.id,
                            result: serde_json::to_value(result)?,
                        })?
                    }
                    Err(e) => {
                        let error = ErrorObject {
                            code: -32000,
                            message: e.to_string(),
                            data: None,
                        };
                        serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error,
                        })?
                    }
                },
                Err(e) => {
                    let error = ErrorObject {
                        code: -32602,
                        message: "Invalid params".to_string(),
                        data: Some(serde_json::Value::String(e.to_string())),
                    };
                    serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error,
                    })?
                }
            },
            "execute" => match serde_json::from_value::<ExecuteParams>(request.params) {
                Ok(params) => match rt.block_on(engine.evaluate(&params.command)) {
                    Ok(output) => {
                        let result = EvaluateResult { output };
                        serde_json::to_string(&Response {
                            id: request.id,
                            result: serde_json::to_value(result)?,
                        })?
                    }
                    Err(e) => {
                        let error = ErrorObject {
                            code: -32000,
                            message: e.to_string(),
                            data: None,
                        };
                        serde_json::to_string(&ErrorResponse {
                            id: request.id,
                            error,
                        })?
                    }
                },
                Err(e) => {
                    let error = ErrorObject {
                        code: -32602,
                        message: "Invalid params".to_string(),
                        data: Some(serde_json::Value::String(e.to_string())),
                    };
                    serde_json::to_string(&ErrorResponse {
                        id: request.id,
                        error,
                    })?
                }
            },
            _ => {
                let error = ErrorObject {
                    code: -32601,
                    message: "Method not found".to_string(),
                    data: None,
                };
                serde_json::to_string(&ErrorResponse {
                    id: request.id,
                    error,
                })?
            }
        };
        println!("{}", response_json);
    }
    Ok(())
}
