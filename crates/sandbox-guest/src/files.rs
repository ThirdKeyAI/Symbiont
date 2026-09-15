//! Declared guest files. Payload bytes follow the command header in input order;
//! one optional output payload follows the final command outcome and streams.
use serde::{Deserialize, Serialize};

pub const MAX_INPUTS: usize = 32;
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_INPUT_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub path: String,
    pub length: u64,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub inputs: Vec<Input>,
    pub output: Option<String>,
    pub max_file_bytes: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub path: String,
    pub length: u64,
    pub sha256: String,
}

pub fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn validate_path(value: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        value.starts_with("/tmp/")
            && value.len() <= 4096
            && !value.contains(['\0', '\n', '\r'])
            && value != crate::INPUT_FILE,
        "guest file paths require nonreserved absolute /tmp paths"
    );
    anyhow::ensure!(
        value
            .split('/')
            .skip(1)
            .all(|part| !part.is_empty() && !matches!(part, "." | "..")),
        "guest file paths must be canonical without traversal"
    );
    Ok(())
}
impl Grant {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.inputs.len() <= MAX_INPUTS && (1..=MAX_FILE_BYTES).contains(&self.max_file_bytes),
            "invalid guest file grant limits"
        );
        let mut paths = Vec::new();
        let mut total = 0u64;
        for input in &self.inputs {
            validate_path(&input.path)?;
            anyhow::ensure!(
                input.length <= self.max_file_bytes && valid_hash(&input.sha256),
                "invalid guest input length or hash"
            );
            total = total
                .checked_add(input.length)
                .ok_or_else(|| anyhow::anyhow!("guest input accounting overflow"))?;
            paths.push(input.path.as_str());
        }
        anyhow::ensure!(total <= MAX_INPUT_BYTES, "guest file inputs exceed 32 MiB");
        if let Some(output) = &self.output {
            validate_path(output)?;
            paths.push(output);
        }
        paths.sort_unstable();
        for (index, path) in paths.iter().enumerate() {
            for other in &paths[index + 1..] {
                anyhow::ensure!(
                    path != other && !other.starts_with(&format!("{path}/")),
                    "overlapping guest file paths"
                );
            }
        }
        Ok(())
    }
}
impl Receipt {
    pub fn validate(&self, grant: &Grant) -> anyhow::Result<()> {
        anyhow::ensure!(
            grant.output.as_deref() == Some(self.path.as_str())
                && self.length <= grant.max_file_bytes
                && valid_hash(&self.sha256),
            "guest file output disagrees with admitted grant"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(path: &str, length: u64) -> Input {
        Input {
            path: path.into(),
            length,
            sha256: "a".repeat(64),
        }
    }
    #[test]
    fn paths_and_aggregate_bounds_are_enforced_before_payload() {
        let mut grant = Grant {
            inputs: vec![input("/tmp/data/a", 1)],
            output: Some("/tmp/result".into()),
            max_file_bytes: MAX_FILE_BYTES,
        };
        grant.validate().unwrap();
        for path in [
            "/tmp",
            "/etc/key",
            "/tmp/../key",
            "/tmp//a",
            "/tmp/a/",
            crate::INPUT_FILE,
        ] {
            assert!(validate_path(path).is_err());
        }
        grant.inputs.push(input("/tmp/data", 1));
        assert!(grant.validate().is_err());
        grant.inputs = vec![
            input("/tmp/a", 1),
            input("/tmp/a-b", 1),
            input("/tmp/a/b", 1),
        ];
        assert!(grant.validate().is_err());
        grant.inputs = vec![
            input("/tmp/a", MAX_FILE_BYTES),
            input("/tmp/b", MAX_FILE_BYTES),
            input("/tmp/c", 1),
        ];
        assert!(grant.validate().is_err());
        grant.inputs = vec![input("/tmp/result", 1)];
        assert!(grant.validate().is_err());
        grant.inputs.clear();
        assert!(Receipt {
            path: "/tmp/other".into(),
            length: 1,
            sha256: "a".repeat(64)
        }
        .validate(&grant)
        .is_err());
        assert!(Receipt {
            path: "/tmp/result".into(),
            length: MAX_FILE_BYTES + 1,
            sha256: "a".repeat(64)
        }
        .validate(&grant)
        .is_err());
    }
}
