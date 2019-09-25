use crate::webauthn::error::Error;
use serde_derive::*;
use serde_cbor::Value;
use std::io::{Cursor, Read};
use byteorder::{ReadBytesExt, BigEndian};
use bytes::Buf;
use std::collections::BTreeMap;
use crate::webauthn::proto::constants::{ECDSA_Y_PREFIX_POSITIVTE, ECDSA_Y_PREFIX_NEGATIVE};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RawAttestationObject {
    auth_data: serde_cbor::Value,
    fmt: String,
    att_stmt: AttestationStatement,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AttestationObject {
    pub auth_data: AuthenticatorData,
    pub raw_auth_data: Vec<u8>,
    pub fmt: String,
    pub att_stmt: AttestationStatement,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AttestationStatement {
    pub alg: i64,
    #[serde(with = "serde_bytes")]
    pub sig: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x5c: Option<serde_cbor::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ecdaa_key_id: Option<serde_cbor::Value>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticatorData {
    pub rp_id_hash: [u8; 32],
    pub flags: u8,
    pub sign_count: u32,
    pub attested_credential_data: Option<AttestedCredentialData>,
    pub extensions: serde_cbor::Value
}

impl AuthenticatorData {
    pub fn from_vec(data: Vec<u8>) -> Result<(Self, Vec<u8>), Error> {
        let mut cursor = Cursor::new(data);

        let mut rp_id_hash = [0u8; 32];
        cursor.read_exact(&mut rp_id_hash)?;

        let flags = cursor.read_u8()?;

        let sign_count = cursor.read_u32::<BigEndian>()?;

        let attested_credential_data = if cursor.remaining() > 16 {
            let mut aaguid = [0u8; 16];
            cursor.read_exact(&mut aaguid)?;

            let length = cursor.read_u16::<BigEndian>()?;

            let mut credential_id = vec![0u8; length as usize];
            cursor.read_exact(&mut credential_id[..])?;

            let mut remaining = vec![0u8; cursor.remaining()];
            cursor.read_exact(&mut remaining[..])?;

            let remaining_value = serde_cbor::from_slice::<serde_cbor::Value>(remaining.as_slice()).map_err(|e| Error::CborError(e))?;

            let credential_public_key = CredentialPublicKey::from_value(remaining_value)?;

            Some(AttestedCredentialData {
                aaguid,
                credential_id,
                credential_public_key,
            })
        } else { None };

        Ok((AuthenticatorData {
            rp_id_hash,
            flags,
            sign_count,
            attested_credential_data,
            extensions: Value::Null
        }, cursor.into_inner()))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AttestedCredentialData {
    pub aaguid: [u8; 16],
    pub credential_id: Vec<u8>,
    pub credential_public_key: CredentialPublicKey,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct CredentialPublicKey {
    pub key_type: i64,
    pub alg: i64,
    pub curve: i64,
    pub coords: Coordinates,
}

impl CredentialPublicKey {
    pub fn from_value(value: serde_cbor::Value) -> Result<Self, Error> {
        let map = match value {
            Value::Map(m) => m,
            _ => { BTreeMap::new() },
        };

        let key_type = map.get(&Value::Integer(1)).map(|val| {
            match val {
                Value::Integer(i) => *i as i64,
                _ => 0i64,
            }
        }).ok_or(Error::Other("Key type missing".to_string()))?;

        let alg = map.get(&Value::Integer(3)).map(|val| {
            match val {
                Value::Integer(i) => *i as i64,
                _ => 0i64,
            }
        }).ok_or(Error::Other("algorithm missing".to_string()))?;

        let curve = map.get(&Value::Integer(-1)).map(|val| {
            match val {
                Value::Integer(i) => *i as i64,
                _ => 0i64,
            }
        }).ok_or(Error::Other("curve missing".to_string()))?;

        let x = map.get(&Value::Integer(-2)).and_then(|val| {
            match val {
                Value::Bytes(i) => {
                    let mut array = [0u8; 32];
                    array.copy_from_slice(&i[0..32]);
                    Some(array)
                },
                _ => None,
            }

        }).ok_or(Error::Other("x coordinate missing".to_string()))?;

        let coords = map.get(&Value::Integer(-3)).and_then(|val| {
            match val {
                Value::Bytes(i) => {
                    let mut array = [0u8; 32];
                    array.copy_from_slice(&i[0..32]);
                    Some(Coordinates::Uncompressed { x, y: array, })
                },

                Value::Bool(b) => {
                    Some(Coordinates::Compressed { x, y: if *b { ECDSA_Y_PREFIX_NEGATIVE } else { ECDSA_Y_PREFIX_POSITIVTE } })
                }
                _ => None,
            }

        }).ok_or(Error::Other("y coordinate missing".to_string()))?;

        Ok(CredentialPublicKey {
            key_type,
            alg,
            curve,
            coords,
        })
    }
}



pub trait Message {
    fn from_base64(string: &String) -> Result<Self, Error> where Self: Sized;
    fn from_bytes(raw_values: &[u8]) -> Result<Self, Error> where Self: Sized;
}

impl Message for AttestationObject {
    fn from_base64(string: &String) -> Result<Self, Error> where Self: Sized {
        let raw_values = base64::decode(string)?;
        Self::from_bytes(raw_values.as_slice())
    }

    fn from_bytes(raw_values: &[u8]) -> Result<Self, Error> where Self: Sized {
        let value = serde_cbor::from_slice::<RawAttestationObject>(raw_values).map_err(|e| Error::CborError(e))?;

        let data = match value.auth_data {
            Value::Bytes(vec) => Ok(vec),
            _ => Err(Error::Other("Cannot proceed without auth data".to_string()))
        }?;

        let (auth_data, raw_auth_data) = AuthenticatorData::from_vec(data)?;

        Ok(AttestationObject {
            auth_data,
            raw_auth_data,
            fmt: value.fmt,
            att_stmt: value.att_stmt
        })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum Coordinates {
    Compressed { x: [u8; 32], y: u8 },
    Uncompressed { x: [u8; 32], y: [u8; 32] },
}