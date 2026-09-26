use crate::{
    Error, Result,
    codec::{Reader, Writer},
};
use aes::Aes128;
use cipher::{BlockEncrypt, KeyInit};
use rand::RngCore;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey, pkcs8::DecodePublicKey};
use sha1::{Digest, Sha1};
use zeroize::Zeroizing;

pub struct OnlineIdentity {
    pub uuid: uuid::Uuid,
    pub username: String,
    access_token: Zeroizing<String>,
}
impl OnlineIdentity {
    pub fn new(uuid: uuid::Uuid, username: String, access_token: String) -> Self {
        Self {
            uuid,
            username,
            access_token: Zeroizing::new(access_token),
        }
    }
}
impl std::fmt::Debug for OnlineIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OnlineIdentity")
            .field("uuid", &self.uuid)
            .field("username", &self.username)
            .field("access_token", &"[REDACTED]")
            .finish()
    }
}

/// Minecraft encrypts the entire framed byte stream with continuous AES-128-CFB8.
/// Separate states per direction; IV and key are both the negotiated random secret.
pub(crate) struct Cfb8 {
    aes: Aes128,
    shift: [u8; 16],
}
impl Cfb8 {
    pub fn new(key: &[u8; 16]) -> Self {
        Self {
            aes: Aes128::new(key.into()),
            shift: *key,
        }
    }
    pub fn apply(&mut self, bytes: &mut [u8], encrypt: bool) {
        for byte in bytes {
            let original = *byte;
            let mut block = self.shift.into();
            self.aes.encrypt_block(&mut block);
            *byte ^= block[0];
            self.shift.copy_within(1.., 0);
            self.shift[15] = if encrypt { *byte } else { original };
        }
    }
}
pub(crate) fn encryption_response(
    r: &mut Reader<'_>,
    identity: Option<&OnlineIdentity>,
) -> Result<(Writer, [u8; 16])> {
    let server_id = r.string(20)?;
    let key_len = r.count(8192)?;
    let public_der = r.take(key_len)?;
    let token_len = r.count(1024)?;
    let token = r.take(token_len)?;
    let authenticate = r.bool()?;
    r.clone().finish()?;
    let key = RsaPublicKey::from_public_key_der(public_der)
        .map_err(|_| Error::Authentication("invalid server RSA public key".into()))?;
    let mut secret = [0; 16];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    if authenticate {
        let identity = identity.ok_or_else(|| {
            Error::Unsupported(
                "online-mode server requires a Microsoft account; choose Sign In before joining"
                    .into(),
            )
        })?;
        let mut hash = Sha1::new();
        hash.update(server_id.as_bytes());
        hash.update(secret);
        hash.update(public_der);
        let digest: [u8; 20] = hash.finalize().into();
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| Error::Authentication("could not create session service client".into()))?;
        let response=client.post("https://sessionserver.mojang.com/session/minecraft/join").json(&serde_json::json!({"accessToken":identity.access_token.as_str(),"selectedProfile":identity.uuid.simple().to_string(),"serverId":signed_sha1_hex(digest)})).send().map_err(|_|Error::Authentication("session service request failed".into()))?;
        if response.status() != reqwest::StatusCode::NO_CONTENT {
            return Err(Error::Authentication(format!(
                "session service rejected join (HTTP {})",
                response.status().as_u16()
            )));
        }
    }
    let encrypted_secret = key
        .encrypt(&mut rand::rngs::OsRng, Pkcs1v15Encrypt, &secret)
        .map_err(|_| Error::Authentication("RSA secret encryption failed".into()))?;
    let encrypted_token = key
        .encrypt(&mut rand::rngs::OsRng, Pkcs1v15Encrypt, token)
        .map_err(|_| Error::Authentication("RSA verify-token encryption failed".into()))?;
    let mut packet = Writer::packet(1);
    packet.varint(encrypted_secret.len() as i32);
    packet.bytes(&encrypted_secret);
    packet.varint(encrypted_token.len() as i32);
    packet.bytes(&encrypted_token);
    Ok((packet, secret))
}
fn signed_sha1_hex(mut bytes: [u8; 20]) -> String {
    let negative = bytes[0] & 0x80 != 0;
    if negative {
        for byte in &mut bytes {
            *byte = !*byte;
        }
        for byte in bytes.iter_mut().rev() {
            let (v, overflow) = byte.overflowing_add(1);
            *byte = v;
            if !overflow {
                break;
            }
        }
    }
    let hex = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let hex = hex.trim_start_matches('0');
    let hex = if hex.is_empty() { "0" } else { hex };
    format!("{}{hex}", if negative { "-" } else { "" })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn minecraft_signed_hash_examples() {
        for (name, expected) in [
            ("Notch", "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48"),
            ("jeb_", "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1"),
            ("simon", "88e16a1019277b15d58faf0541e11910eb756f6"),
        ] {
            let bytes: [u8; 20] = Sha1::digest(name.as_bytes()).into();
            assert_eq!(signed_sha1_hex(bytes), expected);
        }
    }
    #[test]
    fn cfb8_nist_vector_and_chunk_boundaries() {
        // NIST SP800-38A F.3.7 CFB8-AES128 uses a separate IV; set it explicitly here.
        let key = [
            0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf,
            0x4f, 0x3c,
        ];
        let iv = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
        let plain = [
            0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93,
            0x17, 0x2a,
        ];
        let mut crypt = Cfb8::new(&key);
        crypt.shift = iv;
        let mut bytes = plain;
        crypt.apply(&mut bytes, true);
        assert_eq!(
            bytes,
            [
                0x3b, 0x79, 0x42, 0x4c, 0x9c, 0x0d, 0xd4, 0x36, 0xba, 0xce, 0x9e, 0x0e, 0xd4, 0x58,
                0x6a, 0x4f
            ]
        );
        let mut decrypt = Cfb8::new(&key);
        decrypt.shift = iv;
        for slice in bytes.chunks_mut(3) {
            decrypt.apply(slice, false);
        }
        assert_eq!(bytes, plain);
    }
    #[test]
    fn identity_debug_redacts_token() {
        assert!(
            !format!(
                "{:?}",
                OnlineIdentity::new(uuid::Uuid::nil(), "Tester".into(), "secret_token".into())
            )
            .contains("secret_token")
        );
    }
    #[test]
    fn rsa_handshake_encrypts_secret_and_server_token() {
        use rsa::{RsaPrivateKey, pkcs8::EncodePublicKey};
        let private = RsaPrivateKey::new(&mut rand::rngs::OsRng, 1024).unwrap();
        let der = private.to_public_key().to_public_key_der().unwrap();
        let mut request = Writer::default();
        request.string("");
        request.varint(der.as_bytes().len() as i32);
        request.bytes(der.as_bytes());
        request.varint(4);
        request.bytes(&[1, 2, 3, 4]);
        request.bool(false);
        let (response, secret) = encryption_response(&mut Reader::new(&request.0), None).unwrap();
        let mut r = Reader::new(&response.0);
        assert_eq!(r.varint().unwrap(), 1);
        let n = r.count(1024).unwrap();
        let decoded = private
            .decrypt(Pkcs1v15Encrypt, r.take(n).unwrap())
            .unwrap();
        assert_eq!(decoded, secret);
        let n = r.count(1024).unwrap();
        let token = private
            .decrypt(Pkcs1v15Encrypt, r.take(n).unwrap())
            .unwrap();
        assert_eq!(token, [1, 2, 3, 4]);
        r.finish().unwrap();
    }
}
