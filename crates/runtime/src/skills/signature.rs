//! Versioned skill signatures binding authority, identity and every file digest.
use super::{
    files::SkillDirectory,
    loader::{skill_name, verification_domain},
};
use schemapin::{
    crypto::{self, KeyManager},
    skill::SkillSignature,
};
use sha2::{Digest, Sha256};
use std::{io, path::Path};

/// Distinct from the legacy SchemaPin root-only skill signature format.
pub const FORMAT: &str = "symbi-skill-v2";

/// Compact JSON array: format, normalized domain, skill name, key id, timestamp,
/// then a lexicographically ordered path -> SHA-256(content) object.
pub(super) fn payload(sig: &SkillSignature) -> io::Result<Vec<u8>> {
    serde_json::to_vec(&(
        &sig.schemapin_version,
        &sig.domain,
        &sig.skill_name,
        &sig.signer_kid,
        &sig.signed_at,
        &sig.file_manifest,
    ))
    .map_err(io::Error::other)
}

pub(super) fn digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

/// Sign one bounded, confined snapshot and atomically replace its signature file.
/// The caller owns the publishing directory and private key. Verification still
/// re-captures current content; a concurrent edit cannot inherit this signature.
pub fn sign_skill(path: &Path, private_key_pem: &str, domain: &str) -> io::Result<SkillSignature> {
    let directory = SkillDirectory::open(path)?;
    let snapshot = directory.verification_snapshot()?;
    let captured = SkillDirectory::open(snapshot.path())?;
    let content = captured.read_text(Path::new("SKILL.md"), 16 * 1024 * 1024)?;
    let key = KeyManager::load_private_key_pem(private_key_pem).map_err(io::Error::other)?;
    let public = KeyManager::export_public_key_pem(&key.public_key()).map_err(io::Error::other)?;
    let (manifest, content_bytes) = captured.signature_manifest_with_size()?;
    let mut signature = SkillSignature {
        schemapin_version: FORMAT.into(),
        skill_name: skill_name(&content, path),
        skill_hash: String::new(),
        signature: String::new(),
        signed_at: chrono::Utc::now().to_rfc3339(),
        domain: verification_domain(domain)?,
        signer_kid: crypto::calculate_key_id(&public).map_err(io::Error::other)?,
        file_manifest: manifest,
    };
    let bytes = payload(&signature)?;
    signature.skill_hash = digest(&bytes);
    signature.signature = crypto::sign_data(private_key_pem, &bytes).map_err(io::Error::other)?;
    let encoded = serde_json::to_vec_pretty(&signature).map_err(io::Error::other)?;
    validate_signed_size(signature.file_manifest.len(), content_bytes, encoded.len())?;
    directory.write_signature(&encoded)?;
    Ok(signature)
}

fn validate_signed_size(
    files: usize,
    content_bytes: u64,
    signature_bytes: usize,
) -> io::Result<()> {
    if files >= 1_000
        || signature_bytes > 1024 * 1024
        || content_bytes.saturating_add(signature_bytes as u64) > 64 * 1024 * 1024
    {
        return Err(io::Error::other(
            "signed bundle exceeds verification limits including .schemapin.sig",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_reserves_space_for_the_signature_it_adds() {
        let max = 64 * 1024 * 1024;
        assert!(validate_signed_size(999, max - 100, 100).is_ok());
        assert!(validate_signed_size(1, max - 100, 101).is_err());
        assert!(validate_signed_size(1, 0, 1024 * 1024 + 1).is_err());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("SKILL.md"), "safe").unwrap();
        for i in 0..999 {
            std::fs::write(dir.path().join(format!("file-{i}")), "").unwrap();
        }
        let key = crypto::generate_key_pair().unwrap();
        assert!(sign_skill(dir.path(), &key.private_key_pem, "example.test").is_err());
        assert!(!dir.path().join(".schemapin.sig").exists());
        std::fs::remove_file(dir.path().join("file-998")).unwrap();
        sign_skill(dir.path(), &key.private_key_pem, "example.test").unwrap();
        let discovery = schemapin::discovery::build_well_known_response(
            &key.public_key_pem,
            None,
            vec![],
            "1.3",
        );
        assert!(matches!(
            super::super::loader::SkillLoader::new(Default::default()).verify_skill_with_discovery(
                dir.path(),
                &discovery,
                "example.test"
            ),
            super::super::loader::SignatureStatus::Verified { .. }
                | super::super::loader::SignatureStatus::Pinned { .. }
        ));
    }

    #[test]
    fn long_paths_cannot_publish_an_unreadable_signature_envelope() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("SKILL.md"), "safe").unwrap();
        let mut nested = dir.path().to_owned();
        for _ in 0..5 {
            nested.push("d".repeat(200));
        }
        std::fs::create_dir_all(&nested).unwrap();
        for i in 0..998 {
            std::fs::write(nested.join(format!("file-{i}")), "").unwrap();
        }
        let key = crypto::generate_key_pair().unwrap();
        let error = sign_skill(dir.path(), &key.private_key_pem, "example.test").unwrap_err();
        assert!(error.to_string().contains("including .schemapin.sig"));
        assert!(!dir.path().join(".schemapin.sig").exists());
    }
}
