//! Canonical serialization, object identity, and signatures. Spec §0, §8.3.
//!
//! The authoritative form of every fabric object is its canonical JSON (JCS,
//! RFC 8785). An object's `id` is `<prefix>:<hex sha256>` of the canonical
//! body with `id` and `sig` removed (SI-1); the Ed25519 signature covers those
//! same bytes (SI-2). Verification always recomputes from raw JSON so unknown
//! fields are preserved and hashed (§0 extensibility).

use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::de::{DeserializeSeed, Error as _, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::fmt;

const JCS_INTEGER_LIMIT: u64 = 1_u64 << 53;

#[derive(Debug, thiserror::Error)]
pub enum CanonError {
    #[error("object body must be a JSON object")]
    NotAnObject,
    #[error("value outside the ASF JCS input domain at {path}: {detail}")]
    InputDomain { path: String, detail: &'static str },
    #[error("raw fabric JSON rejected before value construction: {0}")]
    RawJson(String),
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

fn w12_number_is_in_domain(number: &serde_json::Number) -> bool {
    if let Some(value) = number.as_i64() {
        value > -(JCS_INTEGER_LIMIT as i64) && value < JCS_INTEGER_LIMIT as i64
    } else {
        // Every allowed positive integer is below 2^53 and therefore also
        // fits i64. This branch contains only larger u64 values or floats,
        // including integer-shaped floats such as `1.0`.
        false
    }
}

fn w12_validate_value(value: &Value, path: &str) -> Result<(), CanonError> {
    match value {
        Value::Number(number) if !w12_number_is_in_domain(number) => {
            let detail = if number.is_f64() {
                "floats are forbidden"
            } else {
                "integer absolute value must be less than 2^53"
            };
            Err(CanonError::InputDomain {
                path: path.to_string(),
                detail,
            })
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                w12_validate_value(child, &format!("{path}/{index}"))?;
            }
            Ok(())
        }
        Value::Object(values) => {
            for (key, child) in values {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                w12_validate_value(child, &format!("{path}/{escaped}"))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Enforce the ASF subset of RFC 8785's input domain (spec §0/A16).
///
/// This accepts only integers whose absolute value is strictly below 2^53;
/// floats are forbidden recursively. Duplicate names cannot exist in a
/// `Value`, so raw fabric JSON must enter through [`parse_fabric_json`].
pub fn validate_jcs_domain(value: &Value) -> Result<(), CanonError> {
    w12_validate_value(value, "")
}

fn w12_duplicate_key(object: &Map<String, Value>, key: &str) -> bool {
    object.contains_key(key)
}

fn w12_raw_number_is_in_domain(token: &str) -> bool {
    if token.contains(['.', 'e', 'E']) {
        return false;
    }
    if token.starts_with('-') {
        // A syntactically negative token parses to <= 0 (`-0` becomes 0),
        // so only the lower absolute-value boundary is meaningful here.
        token
            .parse::<i64>()
            .is_ok_and(|value| value > -(JCS_INTEGER_LIMIT as i64))
    } else {
        token
            .parse::<u64>()
            .is_ok_and(|value| value < JCS_INTEGER_LIMIT)
    }
}

fn w12_validate_raw_numbers(raw: &str) -> Result<(), CanonError> {
    let bytes = raw.as_bytes();
    let mut in_string = false;
    let mut escaped = false;
    let mut number_start = None;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }

        if let Some(start) = number_start {
            if matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | b',' | b']' | b'}') {
                let token = &raw[start..index];
                if !w12_raw_number_is_in_domain(token) {
                    return Err(CanonError::RawJson(format!(
                        "number `{token}` is not an integer with absolute value less than 2^53"
                    )));
                }
                number_start = None;
            }
            continue;
        }

        if byte == b'"' {
            in_string = true;
        } else if byte == b'-' || byte.is_ascii_digit() {
            number_start = Some(index);
        }
    }

    if let Some(start) = number_start {
        let token = &raw[start..];
        if !w12_raw_number_is_in_domain(token) {
            return Err(CanonError::RawJson(format!(
                "number `{token}` is not an integer with absolute value less than 2^53"
            )));
        }
    }
    Ok(())
}

struct FabricValueSeed;

impl<'de> DeserializeSeed<'de> for FabricValueSeed {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for FabricValueSeed {
    type Value = Value;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JSON in the ASF JCS input domain")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let number = serde_json::Number::from(value);
        if w12_number_is_in_domain(&number) {
            Ok(Value::Number(number))
        } else {
            Err(E::custom("integer absolute value must be less than 2^53"))
        }
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let number = serde_json::Number::from(value);
        if w12_number_is_in_domain(&number) {
            Ok(Value::Number(number))
        } else {
            Err(E::custom("integer absolute value must be less than 2^53"))
        }
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        // The lexical pass has already rejected decimal/exponent syntax and
        // oversized integers. serde_json uniquely routes integer token `-0`
        // here; JCS canonicalizes it to `0`, so preserve that valid integer.
        if value == 0.0 && value.is_sign_negative() {
            Ok(Value::Number(serde_json::Number::from(0)))
        } else {
            Err(E::custom("floats are forbidden in fabric objects"))
        }
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(Value::String(value.to_string()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(Value::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(Value::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(self)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(FabricValueSeed)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if w12_duplicate_key(&values, &key) {
                return Err(A::Error::custom(format!("duplicate object name `{key}`")));
            }
            let value = object.next_value_seed(FabricValueSeed)?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

/// Parse one raw fabric object without losing duplicate-name evidence.
///
/// Unrelated JSON protocols and corpus inputs are not fabric objects and do
/// not use this parser. Every persisted signed object/event ingress does.
pub fn parse_fabric_json(raw: &str) -> Result<Value, CanonError> {
    w12_validate_raw_numbers(raw)?;
    let mut deserializer = serde_json::Deserializer::from_str(raw);
    let value = FabricValueSeed
        .deserialize(&mut deserializer)
        .map_err(|error| CanonError::RawJson(error.to_string()))?;
    deserializer
        .end()
        .map_err(|error| CanonError::RawJson(error.to_string()))?;
    Ok(value)
}

/// Canonical (RFC 8785) bytes of JSON in the ASF §0 input domain.
pub fn jcs_bytes(value: &Value) -> Result<Vec<u8>, CanonError> {
    validate_jcs_domain(value)?;
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
    // Validate the complete object, not only the hash body: §0 constrains
    // every fabric field, including unknown extensions that are preserved.
    validate_jcs_domain(raw)?;
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
    // Strict verification rejects non-canonical S values and weak/small-order
    // components. Object ids exclude signatures, so the former behavior was
    // not directly forgeable, but exported ledgers must have one canonical
    // verification boundary (RF-4).
    vk.verify_strict(&bytes, &Signature::from_bytes(&sig_bytes))?;
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
        // Key ordering is by UTF-16 code units. This generic serializer test
        // deliberately stays inside ASF's stricter integer-only input domain;
        // RFC 8785's float support is not fabric-object conformance.
        let v: Value =
            serde_json::from_str(r#"{"b":2,"a":1,"nested":{"y":[15,30],"x":"€"}}"#).unwrap();
        let out = String::from_utf8(jcs_bytes(&v).unwrap()).unwrap();
        assert_eq!(out, r#"{"a":1,"b":2,"nested":{"x":"€","y":[15,30]}}"#);
    }

    fn legacy_seal_without_domain_check(
        prefix: &str,
        obj: Map<String, Value>,
        key: &SigningKey,
    ) -> Map<String, Value> {
        let bytes = serde_json_canonicalizer::to_vec(&Value::Object(obj.clone())).unwrap();
        let mut sealed = obj;
        sealed.insert(
            "id".into(),
            Value::String(format!("{prefix}:{}", hex::encode(Sha256::digest(&bytes)))),
        );
        sealed.insert(
            "sig".into(),
            serde_json::json!({
                "key_id": key_id(&key.verifying_key()),
                "alg": "ed25519",
                "value": hex::encode(key.sign(&bytes).to_bytes()),
            }),
        );
        sealed
    }

    #[test]
    fn jcs_domain_accepts_exact_integer_boundaries_without_changing_signed_bytes() {
        let key = SigningKey::from_bytes(&[7; 32]);
        for token in ["9007199254740991", "-9007199254740991", "-0"] {
            assert!(w12_raw_number_is_in_domain(token), "{token}");
        }
        let body =
            parse_fabric_json(r#"{"nested":{"min":-9007199254740991},"max":9007199254740991}"#)
                .unwrap()
                .as_object()
                .unwrap()
                .clone();

        let current = seal("test", body.clone(), &key).unwrap();
        let legacy = legacy_seal_without_domain_check("test", body, &key);
        assert_eq!(current, legacy, "W-12 must not change valid signed bytes");
        verify(&Value::Object(current), &key.verifying_key()).unwrap();

        let negative_zero = parse_fabric_json(r#"{"n":-0}"#).unwrap();
        assert_eq!(String::from_utf8(jcs_bytes(&negative_zero).unwrap()).unwrap(), r#"{"n":0}"#);
        assert!(parse_fabric_json(
            r#"{"escaped":"\"9007199254740992","number-shaped":"1.0"}"#
        )
        .is_ok());
    }

    #[test]
    fn jcs_domain_rejects_collisions_floats_and_duplicates_without_signing_or_accepting() {
        let key = SigningKey::from_bytes(&[9; 32]);
        for token in ["9007199254740992", "-9007199254740992", "1.0", "1e0"] {
            assert!(!w12_raw_number_is_in_domain(token), "{token}");
        }
        let invalid_values = [
            serde_json::json!({"n": 9_007_199_254_740_992_u64}),
            serde_json::json!({"n": -9_007_199_254_740_992_i64}),
            serde_json::json!({"n": 1.0}),
            serde_json::json!({"nested": [{"n": 1.5}]}),
        ];
        for value in invalid_values {
            let body = value.as_object().unwrap().clone();
            let result = seal("test", body, &key);
            assert!(
                matches!(result, Err(CanonError::InputDomain { .. })),
                "out-of-domain body must not receive an id or signature: {value}"
            );
        }

        for raw_float in [r#"{"n":-0.0}"#, r#"{"n":1e0}"#] {
            assert!(parse_fabric_json(raw_float).is_err());
        }

        // RF-17 reproduction: the pre-W-12 canonicalizer maps these adjacent
        // exact serde integers to identical bytes. Both are now outside the
        // accepted input domain, at construction and verification.
        let left = serde_json::json!({"n": 9_007_199_254_740_992_u64});
        let right = serde_json::json!({"n": 9_007_199_254_740_993_u64});
        assert_eq!(
            serde_json_canonicalizer::to_vec(&left).unwrap(),
            serde_json_canonicalizer::to_vec(&right).unwrap()
        );
        let legacy =
            legacy_seal_without_domain_check("test", left.as_object().unwrap().clone(), &key);
        let mut collision = legacy;
        collision.insert("n".into(), Value::from(9_007_199_254_740_993_u64));
        assert!(matches!(
            verify(&Value::Object(collision), &key.verifying_key()),
            Err(CanonError::InputDomain { .. })
        ));

        for raw in [r#"{"x":1,"x":2}"#, r#"{"outer":{"x":1,"x":2}}"#] {
            assert!(
                matches!(parse_fabric_json(raw), Err(CanonError::RawJson(_))),
                "duplicate names must produce no accepted fabric value"
            );
        }
    }

    #[test]
    fn jcs_language_neutral_vectors_match_reference_outcomes() {
        let vectors: Value =
            serde_json::from_str(include_str!("../../../tests/vectors/jcs-input-domain.json"))
                .unwrap();
        for vector in vectors["vectors"].as_array().unwrap() {
            let name = vector["name"].as_str().unwrap();
            let raw = vector["input"].as_str().unwrap();
            match vector["result"].as_str().unwrap() {
                "accept" => {
                    let value = parse_fabric_json(raw)
                        .unwrap_or_else(|error| panic!("{name} must be accepted: {error}"));
                    let canonical = String::from_utf8(jcs_bytes(&value).unwrap()).unwrap();
                    assert_eq!(canonical, vector["canonical"], "{name}");
                    assert_eq!(sha256_hex(canonical.as_bytes()), vector["sha256"], "{name}");
                }
                "reject" => assert!(
                    parse_fabric_json(raw).is_err(),
                    "{name} must be rejected before canonicalization"
                ),
                result => panic!("unknown vector result {result}"),
            }
        }
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
