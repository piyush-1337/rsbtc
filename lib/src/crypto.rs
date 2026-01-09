use ecdsa::{
    Signature as EcdsaSignature, SigningKey, VerifyingKey,
    signature::{Signer, rand_core::OsRng},
};
use k256::Secp256k1;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Signature(pub EcdsaSignature<Secp256k1>);

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct PublicKey(pub VerifyingKey<Secp256k1>);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PrivateKey(#[serde(with = "signkey_serde")] pub SigningKey<Secp256k1>);

impl PrivateKey {
    pub fn new_key() -> Self {
        Self(SigningKey::random(&mut OsRng))
    }

    pub fn public_key(&self) -> PublicKey {
        PublicKey(*self.0.verifying_key())
    }
}

mod signkey_serde {
    use serde::Deserialize;

    pub fn serialize<S>(
        key: &super::SigningKey<super::Secp256k1>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_bytes(&key.to_bytes())
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<super::SigningKey<super::Secp256k1>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bytes: Vec<u8> = Vec::<u8>::deserialize(deserializer)?;
        Ok(super::SigningKey::from_slice(&bytes).unwrap())
    }
}
