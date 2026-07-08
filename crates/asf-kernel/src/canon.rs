//! Canonical serialization, object identity, and signatures. Spec §0, §8.3.
//!
//! The authoritative form of every fabric object is its canonical JSON (JCS,
//! RFC 8785). An object's `id` is `<prefix>:<hex sha256>` of the canonical
//! body with `id` and `sig` removed (SI-1); the Ed25519 signature covers those
//! same bytes (SI-2). Verification always recomputes from raw JSON so unknown
//! fields are preserved and hashed (§0 extensibility).

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum CanonError {
    #[error("object body must be a JSON object")]
    NotAnObject,
    #[error("canonicalization failed: {0}")]
    Jcs(#[from] serde_json::Error),
    #[error("object has no `{0}` field")]
    MissingField(&'static str),
    #[error("id mismatch: claimed {claimed}, computed {computed}")]
    IdMismatch { claimed: String, computed: String },
    #[error("bad signature encoding: {0}")]
    SigEncoding(String),
    #[error("signature invalid: {0}")]
    SigInvalid(#[from] ed25519_dalek::SignatureError),
    #[error("sig.alg is {0}, expected ed25519")]
    WrongAlg(String),
    #[error("sig.key_id is {claimed}, expected {expected}")]
    WrongKey { claimed: String, expected: String },
}

/// Canonical (RFC 8785) bytes of any JSON value.
pub fn jcs_bytes(value: &Value) -> Result<Vec<u8>, CanonError> {
    Ok(serde_json_canonicalizer::to_vec(value)?)
}

/// The hash body: the object with `id` and `sig` removed (SI-1).
fn hash_body(obj: &Map<String, Value>) -> Map<String, Value> {
    let mut body = obj.clone();
    body.remove("id");
    body.remove("sig");
    body
}

/// Canonical bytes of the hash body — the message that is both hashed for the
/// id and signed (SI-2).
pub fn body_bytes(obj: &Map<String, Value>) -> Result<Vec<u8>, CanonError> {
    jcs_bytes(&Value::Object(hash_body(obj)))
}

/// Compute `<prefix>:<hex sha256(JCS(body ∖ {id, sig}))>`.
pub fn compute_id(prefix: &str, obj: &Map<String, Value>) -> Result<String, CanonError> {
    let bytes = body_bytes(obj)?;
    Ok(format!("{prefix}:{}", hex::encode(Sha256::digest(&bytes))))
}

/// `sha256:<hex>` of arbitrary content (payload hashes, state roots).
pub fn sha256_hex(content: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(content)))
}

/// Key id for a verifying key: `key:<hex sha256(pubkey)>` (first 16 bytes for
/// legibility; the full key is registered alongside).
pub fn key_id(vk: &VerifyingKey) -> String {
    let d = Sha256::digest(vk.as_bytes());
    format!("key:{}", hex::encode(&d[..16]))
}

/// Stamp `id` and `sig` onto an object body. Returns the completed object.
/// The prefix is the id namespace ("man", "evt", "int", …).
pub fn seal(
    prefix: &str,
    mut obj: Map<String, Value>,
    sk: &SigningKey,
) -> Result<Map<String, Value>, CanonError> {
    obj.remove("id");
    obj.remove("sig");
    let bytes = body_bytes(&obj)?;
    let id = format!("{prefix}:{}", hex::encode(Sha256::digest(&bytes)));
    let sig = sk.sign(&bytes);
    obj.insert("id".into(), Value::String(id));
    obj.insert(
        "sig".into(),
        serde_json::json!({
            "key_id": key_id(&sk.verifying_key()),
            "alg": "ed25519",
            "value": hex::encode(sig.to_bytes()),
        }),
    );
    Ok(obj)
}

/// Full verification of a sealed object: recompute id from raw bytes, verify
/// the signature over the same bytes against `vk`, and check the recorded
/// key_id matches `vk`.
pub fn verify(raw: &Value, vk: &VerifyingKey) -> Result<(), CanonError> {
    let obj = raw.as_object().ok_or(CanonError::NotAnObject)?;
    let claimed_id = obj
        .get("id")
        .and_then(Value::as_str)
        .ok_or(CanonError::MissingField("id"))?;
    let prefix = claimed_id.split(':').next().unwrap_or_default();
    let computed = compute_id(prefix, obj)?;
    if computed != claimed_id {
        return Err(CanonError::IdMismatch {
            claimed: claimed_id.into(),
            computed,
        });
    }
    let sig = obj.get("sig").ok_or(CanonError::MissingField("sig"))?;
    let alg = sig.get("alg").and_then(Value::as_str).unwrap_or_default();
    if alg != "ed25519" {
        return Err(CanonError::WrongAlg(alg.into()));
    }
    let claimed_key = sig
        .get("key_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let expected_key = key_id(vk);
    if claimed_key != expected_key {
        return Err(CanonError::WrongKey {
            claimed: claimed_key.into(),
            expected: expected_key,
        });
    }
    let sig_hex = sig
        .get("value")
        .and_then(Value::as_str)
        .ok_or(CanonError::MissingField("sig.value"))?;
    let sig_bytes: [u8; 64] = hex::decode(sig_hex)
        .map_err(|e| CanonError::SigEncoding(e.to_string()))?
        .try_into()
        .map_err(|_| CanonError::SigEncoding("signature must be 64 bytes".into()))?;
    let bytes = body_bytes(obj)?;
    vk.verify(&bytes, &Signature::from_bytes(&sig_bytes))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::OsRng;

    fn sk() -> SigningKey {
        SigningKey::generate(&mut OsRng)
    }

    #[test]
    fn jcs_rfc8785_shapes() {
        // Key ordering is by UTF-16 code units; numbers serialize ES-style.
        let v: Value = serde_json::from_str(r#"{"b":2,"a":1,"nested":{"y":[1.5,1e30],"x":"€"}}"#)
            .unwrap();
        let out = String::from_utf8(jcs_bytes(&v).unwrap()).unwrap();
        assert_eq!(out, r#"{"a":1,"b":2,"nested":{"x":"€","y":[1.5,1e+30]}}"#);
    }

    #[test]
    fn id_excludes_id_and_sig_and_is_stable() {
        let body: Map<String, Value> =
            serde_json::from_str(r#"{"kind":"human","meta":{"label":"josh"}}"#).unwrap();
        let sealed = seal("prin", body.clone(), &sk()).unwrap();
        // A different key produces a different sig but the SAME id (SI-1/SI-2:
        // id covers body only).
        let sealed2 = seal("prin", body, &sk()).unwrap();
        assert_eq!(sealed["id"], sealed2["id"]);
        assert_ne!(sealed["sig"], sealed2["sig"]);
        assert!(sealed["id"].as_str().unwrap().starts_with("prin:"));
    }

    #[test]
    fn unknown_fields_are_hashed() {
        let a: Map<String, Value> = serde_json::from_str(r#"{"kind":"human"}"#).unwrap();
        let mut b = a.clone();
        b.insert("future_field".into(), Value::Bool(true));
        assert_ne!(
            compute_id("prin", &a).unwrap(),
            compute_id("prin", &b).unwrap()
        );
    }

    #[test]
    fn verify_roundtrip_and_tamper_detection() {
        let key = sk();
        let body: Map<String, Value> =
            serde_json::from_str(r#"{"kind":"agent","n":42}"#).unwrap();
        let sealed = seal("prin", body, &key).unwrap();
        let raw = Value::Object(sealed.clone());
        verify(&raw, &key.verifying_key()).unwrap();

        // Tampering with any field breaks the id check.
        let mut tampered = sealed.clone();
        tampered.insert("n".into(), Value::from(43));
        assert!(matches!(
            verify(&Value::Object(tampered), &key.verifying_key()),
            Err(CanonError::IdMismatch { .. })
        ));

        // Recomputing the id but keeping the old sig breaks sig verification.
        let mut resealed = sealed.clone();
        resealed.insert("n".into(), Value::from(43));
        let new_id = compute_id("prin", &resealed).unwrap();
        resealed.insert("id".into(), Value::String(new_id));
        assert!(verify(&Value::Object(resealed), &key.verifying_key()).is_err());

        // Wrong verifying key is rejected via key_id before sig check.
        assert!(matches!(
            verify(&raw, &sk().verifying_key()),
            Err(CanonError::WrongKey { .. })
        ));
    }

    #[test]
    fn unknown_fields_survive_reserialize() {
        // Round-trip through serde_json::Value preserves unknown fields;
        // verification of an object with fields we don't model still passes.
        let key = sk();
        let body: Map<String, Value> = serde_json::from_str(
            r#"{"kind":"service","x_extension":{"deep":["unknown",1,true]}}"#,
        )
        .unwrap();
        let sealed = seal("prin", body, &key).unwrap();
        let json = serde_json::to_string(&sealed).unwrap();
        let reparsed: Value = serde_json::from_str(&json).unwrap();
        verify(&reparsed, &key.verifying_key()).unwrap();
    }
}
