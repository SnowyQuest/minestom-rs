//! Production authentication & cryptography for `minestom-rs`:
//! - **Online Mode (`MojangCrypt`)**: RSA-1024 keypair generation, PKCS#1 v1.5 encryption/decryption,
//!   streaming AES-128-CFB8 (`CFB/8`) socket cipher, two's-complement signed SHA-1 server hash, and
//!   async Mojang `hasJoined` session verification (`reqwest` + `rustls`).
//! - **Velocity Modern Forwarding (`velocity:player_info`)**: Constant-time `HMAC-SHA256` verification
//!   (version `1`) extracting client IP, UUID, username, and skin properties.
//! - **`BungeeCord` / `BungeeGuard` Forwarding**: `\0`-delimited handshake forwarding with strict single
//!   `bungeeguard-token` verification.
//! - **Token-Bucket `PacketRateLimiter`**: Per-connection O(1) rate limiting against packet floods.

use std::collections::BTreeSet;
use std::net::IpAddr;
use std::time::Instant;

use aes::Aes128;
use aes::cipher::{Array, BlockCipherEncrypt, KeyInit};
use bytes::{BufMut, BytesMut};
use hmac::{Hmac, Mac};
use minestom_protocol::login::{GameProfile, ProfileProperty};
use minestom_protocol::{Decode, Encode, read_string, read_var_int, write_string, write_var_int};
use rand::rngs::SysRng;
use rsa::pkcs8::{DecodePublicKey, EncodePublicKey};
use rsa::{PrivateKey, PublicKey};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use sha2::Sha256;
use uuid::Uuid;

use crate::NetworkError;

/// Default Mojang Yggdrasil session verification endpoint.
pub const DEFAULT_MOJANG_HAS_JOINED_URL: &str =
    "https://sessionserver.mojang.com/session/minecraft/hasJoined";

/// Velocity modern login-plugin channel identifier.
pub const VELOCITY_PLAYER_INFO_CHANNEL: &str = "velocity:player_info";

/// Supported Velocity modern forwarding protocol version (`1`).
pub const VELOCITY_FORWARDING_VERSION: i32 = 1;

type HmacSha256 = Hmac<Sha256>;

///Stateful streaming AES-128-CFB8 (`CFB/8`) encryptor (NIST SP 800-38A §6.3).
///
/// Processes arbitrary byte slices across TCP writes without requiring 16-byte block alignment.
#[derive(Clone)]
pub struct Aes128Cfb8Encryptor {
    cipher: Aes128,
    shift_register: [u8; 16],
}

impl std::fmt::Debug for Aes128Cfb8Encryptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Aes128Cfb8Encryptor")
            .finish_non_exhaustive()
    }
}

impl Aes128Cfb8Encryptor {
    /// Initializes an AES-128-CFB8 encryptor where both `key` and `iv` equal the 16-byte `shared_secret`.
    #[must_use]
    pub fn new(shared_secret: &[u8; 16]) -> Self {
        let key = Array::from(*shared_secret);
        Self {
            cipher: Aes128::new(&key),
            shift_register: *shared_secret,
        }
    }

    /// Encrypts `bytes` in-place byte-by-byte, advancing the 16-byte CFB-8 shift register.
    pub fn encrypt_in_place(&mut self, bytes: &mut [u8]) {
        for b in bytes {
            let mut block = Array::from(self.shift_register);
            self.cipher.encrypt_block(&mut block);
            let cipher_byte = *b ^ block[0];
            self.shift_register.copy_within(1..16, 0);
            self.shift_register[15] = cipher_byte;
            *b = cipher_byte;
        }
    }
}

/// Stateful streaming AES-128-CFB8 (`CFB/8`) decryptor (NIST SP 800-38A §6.3).
///
/// Processes arbitrary partial TCP read chunks seamlessly.
#[derive(Clone)]
pub struct Aes128Cfb8Decryptor {
    cipher: Aes128,
    shift_register: [u8; 16],
}

impl std::fmt::Debug for Aes128Cfb8Decryptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Aes128Cfb8Decryptor")
            .finish_non_exhaustive()
    }
}

impl Aes128Cfb8Decryptor {
    /// Initializes an AES-128-CFB8 decryptor where both `key` and `iv` equal the 16-byte `shared_secret`.
    #[must_use]
    pub fn new(shared_secret: &[u8; 16]) -> Self {
        let key = Array::from(*shared_secret);
        Self {
            cipher: Aes128::new(&key),
            shift_register: *shared_secret,
        }
    }

    /// Decrypts `bytes` in-place byte-by-byte, advancing the 16-byte CFB-8 shift register.
    pub fn decrypt_in_place(&mut self, bytes: &mut [u8]) {
        for b in bytes {
            let cipher_byte = *b;
            let mut block = Array::from(self.shift_register);
            self.cipher.encrypt_block(&mut block);
            let plain_byte = cipher_byte ^ block[0];
            self.shift_register.copy_within(1..16, 0);
            self.shift_register[15] = cipher_byte;
            *b = plain_byte;
        }
    }
}

/// Server RSA-1024 keypair used for Minecraft Online-Mode login encryption (`MojangCrypt`).
#[derive(Clone)]
pub struct ServerEncryptionKeys {
    private_key: PrivateKey,
    public_key_der: Vec<u8>,
}

impl std::fmt::Debug for ServerEncryptionKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerEncryptionKeys")
            .field("public_key_der_len", &self.public_key_der.len())
            .finish_non_exhaustive()
    }
}

impl ServerEncryptionKeys {
    /// Generates a fresh 1024-bit RSA keypair using the OS CSPRNG.
    pub fn generate() -> Result<Self, NetworkError> {
        let private_key = PrivateKey::new(&mut SysRng, 1024)
            .map_err(|e| NetworkError::Auth(format!("RSA-1024 key generation failed: {e}")))?;
        let public_key_der = private_key
            .to_public_key()
            .to_public_key_der()
            .map_err(|e| NetworkError::Auth(format!("SPKI DER encoding failed: {e}")))?
            .into_vec();
        Ok(Self {
            private_key,
            public_key_der,
        })
    }

    /// X.509 `SubjectPublicKeyInfo` DER-encoded public key bytes sent in `EncryptionRequestPacket`.
    #[must_use]
    pub fn public_key_der(&self) -> &[u8] {
        &self.public_key_der
    }

    /// Generates a random 4-byte challenge nonce (`verify_token`).
    #[must_use]
    pub fn generate_verify_token() -> [u8; 4] {
        rand::random::<[u8; 4]>()
    }

    /// Decrypts RSA-1024 PKCS#1 v1.5 ciphertext bytes.
    pub fn decrypt_pkcs1v15(&self, ciphertext: &[u8]) -> Result<Vec<u8>, NetworkError> {
        self.private_key
            .pkcs1v15_decrypt(ciphertext)
            .map_err(|_| NetworkError::Auth("RSA PKCS#1 v1.5 decryption failed".to_owned()))
    }

    /// Decrypts and validates the client's `shared_secret` (`16` bytes) and `verify_token`.
    pub fn verify_encryption_response(
        &self,
        expected_nonce: &[u8],
        encrypted_shared_secret: &[u8],
        encrypted_verify_token: &[u8],
    ) -> Result<[u8; 16], NetworkError> {
        let token = self.decrypt_pkcs1v15(encrypted_verify_token)?;
        if token != expected_nonce {
            return Err(NetworkError::Auth("verify token mismatch".to_owned()));
        }
        let secret_vec = self.decrypt_pkcs1v15(encrypted_shared_secret)?;
        let secret: [u8; 16] = secret_vec
            .try_into()
            .map_err(|_| NetworkError::Auth("shared secret must be 16 bytes".to_owned()))?;
        Ok(secret)
    }
}

/// Encrypts `plaintext` with an X.509 `SubjectPublicKeyInfo` DER RSA public key using PKCS#1 v1.5
/// (used by Online-Mode client tests and tooling).
pub fn encrypt_rsa_pkcs1v15(
    public_key_der: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, NetworkError> {
    let pub_key = PublicKey::from_public_key_der(public_key_der)
        .map_err(|e| NetworkError::Auth(format!("invalid RSA SPKI DER public key: {e}")))?;
    pub_key
        .pkcs1v15_encrypt(plaintext)
        .map_err(|e| NetworkError::Auth(format!("RSA PKCS#1 v1.5 encrypt failed: {e}")))
}

/// Computes Mojang's non-standard two's-complement signed SHA-1 hex digest
/// (`new BigInteger(sha1(server_id || shared_secret || public_key_der)).toString(16)`).
#[must_use]
pub fn mojang_sha1_server_id(
    server_id: &str,
    shared_secret: &[u8],
    public_key_der: &[u8],
) -> String {
    let mut hasher = Sha1::new();
    hasher.update(server_id.as_bytes());
    hasher.update(shared_secret);
    hasher.update(public_key_der);
    let digest: [u8; 20] = hasher.finalize().into();
    sha1_bytes_to_mojang_hex(digest)
}

/// Formats a raw 20-byte SHA-1 digest using Java `BigInteger(bytes).toString(16)` signed hex rules.
#[must_use]
pub fn sha1_bytes_to_mojang_hex(mut bytes: [u8; 20]) -> String {
    let negative = (bytes[0] & 0x80) != 0;
    if negative {
        let mut carry = true;
        for b in bytes.iter_mut().rev() {
            *b = !*b;
            if carry {
                let (next, overflow) = b.overflowing_add(1);
                *b = next;
                carry = overflow;
            }
        }
    }

    let mut hex = String::with_capacity(41);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{b:02x}");
    }
    let trimmed = hex.trim_start_matches('0');
    if trimmed.is_empty() {
        "0".to_owned()
    } else if negative {
        format!("-{trimmed}")
    } else {
        trimmed.to_owned()
    }
}

#[derive(Debug, Deserialize)]
struct MojangHasJoinedJson {
    id: String,
    name: String,
    #[serde(default)]
    properties: Vec<MojangPropertyJson>,
}

#[derive(Debug, Deserialize)]
struct MojangPropertyJson {
    name: String,
    value: String,
    #[serde(default)]
    signature: Option<String>,
}

/// Asynchronous HTTP/HTTPS client for Mojang's `session/minecraft/hasJoined` endpoint.
#[derive(Debug, Clone)]
pub struct MojangSessionClient {
    http: reqwest::Client,
    endpoint_url: String,
    prevent_proxy_connections: bool,
}

impl Default for MojangSessionClient {
    fn default() -> Self {
        Self::new(DEFAULT_MOJANG_HAS_JOINED_URL, false)
    }
}

impl MojangSessionClient {
    /// Creates a [`MojangSessionClient`] targeting `endpoint_url`.
    #[must_use]
    pub fn new(endpoint_url: impl Into<String>, prevent_proxy_connections: bool) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            http,
            endpoint_url: endpoint_url.into(),
            prevent_proxy_connections,
        }
    }

    /// Verifies that `username` joined the server with `server_hash`, returning the authenticated
    /// [`GameProfile`] (including signed `"textures"` skin properties!).
    pub async fn verify_has_joined(
        &self,
        username: &str,
        server_hash: &str,
        client_ip: Option<IpAddr>,
    ) -> Result<GameProfile, NetworkError> {
        let sep = if self.endpoint_url.contains('?') {
            '&'
        } else {
            '?'
        };
        let mut url = format!(
            "{}{sep}username={username}&serverId={server_hash}",
            self.endpoint_url
        );
        if self.prevent_proxy_connections
            && let Some(ip) = client_ip
        {
            use std::fmt::Write as _;
            let _ = write!(&mut url, "&ip={ip}");
        }

        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| NetworkError::Auth(format!("session server request failed: {e}")))?;

        if resp.status().as_u16() == 204 || !resp.status().is_success() {
            return Err(NetworkError::Auth(format!(
                "unverified Mojang session (HTTP {})",
                resp.status()
            )));
        }

        let parsed: MojangHasJoinedJson = resp
            .json()
            .await
            .map_err(|e| NetworkError::Auth(format!("invalid session server JSON: {e}")))?;

        let uuid = parse_flexible_uuid(&parsed.id)?;
        let properties = parsed
            .properties
            .into_iter()
            .map(|p| ProfileProperty {
                name: p.name,
                value: p.value,
                signature: p.signature,
            })
            .collect();

        Ok(GameProfile {
            uuid,
            username: parsed.name,
            properties,
        })
    }
}

/// Parses either a standard hyphenated UUID or Mojang's 32-hex-digit undashed UUID.
pub fn parse_flexible_uuid(raw: &str) -> Result<Uuid, NetworkError> {
    Uuid::parse_str(raw)
        .map_err(|e| NetworkError::Auth(format!("invalid player UUID `{raw}`: {e}")))
}

/// Verified player identity and real client IP extracted from Velocity or BungeeCord forwarding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForwardedPlayer {
    /// Real client IP address reported by the proxy.
    pub client_ip: IpAddr,
    /// Authenticated [`GameProfile`] (UUID, username, skin properties).
    pub profile: GameProfile,
}

/// Verifies a `velocity:player_info` `ClientLoginPluginResponsePacket` payload using constant-time
/// `HMAC-SHA256(secret, body)` and forwarding version `1` (`Auth.Velocity` in Minestom).
pub fn verify_velocity_player_info(
    secret: &[u8],
    payload: &[u8],
) -> Result<ForwardedPlayer, NetworkError> {
    if payload.len() < 33 {
        return Err(NetworkError::Auth(
            "Velocity forwarding payload shorter than 32-byte HMAC".to_owned(),
        ));
    }
    let (sig, body) = payload.split_at(32);
    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|_| NetworkError::Auth("invalid Velocity HMAC secret".to_owned()))?;
    mac.update(body);
    mac.verify_slice(sig)
        .map_err(|_| NetworkError::Auth("Velocity HMAC-SHA256 signature mismatch".to_owned()))?;

    let mut cursor = body;
    let version = read_var_int(&mut cursor)?;
    if version != VELOCITY_FORWARDING_VERSION {
        return Err(NetworkError::Auth(format!(
            "unsupported Velocity forwarding version {version}"
        )));
    }
    let ip_str = read_string(&mut cursor, 128)?;
    let client_ip: IpAddr = ip_str.parse().map_err(|e| {
        NetworkError::Auth(format!("invalid Velocity forwarded IP `{ip_str}`: {e}"))
    })?;
    let profile = GameProfile::decode(&mut cursor)?;
    Ok(ForwardedPlayer { client_ip, profile })
}

/// Encodes a valid `velocity:player_info` v1 payload signed with `HMAC-SHA256(secret)`
/// (useful for proxy simulations and integration tests).
pub fn encode_velocity_player_info(
    secret: &[u8],
    client_ip: IpAddr,
    profile: &GameProfile,
) -> Result<Vec<u8>, NetworkError> {
    let mut body = BytesMut::new();
    write_var_int(&mut body, VELOCITY_FORWARDING_VERSION);
    write_string(&mut body, &client_ip.to_string(), 128)?;
    profile.encode(&mut body)?;

    let mut mac = HmacSha256::new_from_slice(secret)
        .map_err(|_| NetworkError::Auth("invalid Velocity HMAC secret".to_owned()))?;
    mac.update(&body);
    let sig = mac.finalize().into_bytes();

    let mut out = BytesMut::with_capacity(32 + body.len());
    out.put_slice(&sig);
    out.put_slice(&body);
    Ok(out.to_vec())
}

/// Parses and validates `BungeeCord` / `BungeeGuard` `\0`-delimited `server_address` forwarding
/// (`host\0client_ip\0uuid[\0properties_json]`), matching `HandshakeListener.java` in Minestom!
pub fn parse_bungee_handshake(
    raw_address: &str,
    guard_tokens: Option<&BTreeSet<String>>,
) -> Result<(String, IpAddr, Uuid, Vec<ProfileProperty>), NetworkError> {
    let parts: Vec<&str> = raw_address.split('\0').collect();
    if parts.len() != 3 && parts.len() != 4 {
        return Err(NetworkError::Auth(
            "invalid BungeeCord handshake field count".to_owned(),
        ));
    }
    if guard_tokens.is_some() && parts.len() != 4 {
        return Err(NetworkError::Auth(
            "missing BungeeGuard properties in handshake".to_owned(),
        ));
    }

    let forwarded_host = parts[0].to_owned();
    let client_ip: IpAddr = parts[1].parse().map_err(|e| {
        NetworkError::Auth(format!(
            "invalid BungeeCord forwarded IP `{}`: {e}",
            parts[1]
        ))
    })?;
    let player_uuid = parse_flexible_uuid(parts[2])?;

    let mut properties = Vec::new();
    if parts.len() == 4 {
        let parsed_props: Vec<MojangPropertyJson> = serde_json::from_str(parts[3])
            .map_err(|e| NetworkError::Auth(format!("invalid BungeeCord properties JSON: {e}")))?;
        let mut found_guard_token = false;
        for p in parsed_props {
            if p.name == "bungeeguard-token" {
                if let Some(allowed) = guard_tokens {
                    if found_guard_token || !allowed.contains(&p.value) {
                        return Err(NetworkError::Auth(
                            "invalid or duplicate BungeeGuard token".to_owned(),
                        ));
                    }
                    found_guard_token = true;
                }
                continue; // Do not forward internal proxy token to game clients
            }
            properties.push(ProfileProperty {
                name: p.name,
                value: p.value,
                signature: p.signature,
            });
        }
        if guard_tokens.is_some() && !found_guard_token {
            return Err(NetworkError::Auth(
                "missing required bungeeguard-token property".to_owned(),
            ));
        }
    }

    Ok((forwarded_host, client_ip, player_uuid, properties))
}

/// Per-connection O(1) token-bucket packet rate limiter protecting against inbound packet floods.
#[derive(Debug, Clone)]
pub struct PacketRateLimiter {
    rate_per_sec: f64,
    burst_capacity: f64,
    tokens: f64,
    last_refill: Instant,
}

impl PacketRateLimiter {
    /// Creates a new [`PacketRateLimiter`] allowing `max_packets_per_second` sustained with `burst` headroom.
    #[must_use]
    pub fn new(max_packets_per_second: u32, burst: u32) -> Self {
        let cap = f64::from(burst.max(1));
        Self {
            rate_per_sec: f64::from(max_packets_per_second.max(1)),
            burst_capacity: cap,
            tokens: cap,
            last_refill: Instant::now(),
        }
    }

    /// Attempts to consume `1` packet token at `now`, returning `false` if the peer exceeded its rate limit!
    pub fn try_acquire(&mut self, now: Instant) -> bool {
        let elapsed = now
            .saturating_duration_since(self.last_refill)
            .as_secs_f64();
        if elapsed > 0.0 {
            self.tokens = (self.tokens + elapsed * self.rate_per_sec).min(self.burst_capacity);
            self.last_refill = now;
        }
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn mojang_sha1_matches_official_wiki_vg_test_vectors() {
        // Official Notch / jeb_ / simon SHA-1 test vectors!
        let mut h1 = Sha1::new();
        h1.update(b"Notch");
        assert_eq!(
            sha1_bytes_to_mojang_hex(h1.finalize().into()),
            "4ed1f46bbe04bc756bcb17c0c7ce3e4632f06a48"
        );

        let mut h2 = Sha1::new();
        h2.update(b"jeb_");
        assert_eq!(
            sha1_bytes_to_mojang_hex(h2.finalize().into()),
            "-7c9d5b0044c130109a5d7b5fb5c317c02b4e28c1"
        );

        let mut h3 = Sha1::new();
        h3.update(b"simon");
        assert_eq!(
            sha1_bytes_to_mojang_hex(h3.finalize().into()),
            "88e16a1019277b15d58faf0541e11910eb756f6"
        );
    }

    #[test]
    fn aes128_cfb8_streaming_chunks_and_rsa1024_roundtrip() {
        let secret = [
            0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf,
            0x4f, 0x3c,
        ];
        let original =
            b"Minecraft 26.2 AES-128-CFB8 streaming packet payload across partial TCP reads!";
        let mut wire = original.to_vec();

        let mut enc = Aes128Cfb8Encryptor::new(&secret);
        enc.encrypt_in_place(&mut wire[..7]);
        enc.encrypt_in_place(&mut wire[7..29]);
        enc.encrypt_in_place(&mut wire[29..]);
        assert_ne!(&wire[..], &original[..]);

        let mut dec = Aes128Cfb8Decryptor::new(&secret);
        dec.decrypt_in_place(&mut wire[..3]);
        dec.decrypt_in_place(&mut wire[3..41]);
        dec.decrypt_in_place(&mut wire[41..]);
        assert_eq!(&wire[..], &original[..]);

        let keys = ServerEncryptionKeys::generate().unwrap();
        let nonce = ServerEncryptionKeys::generate_verify_token();
        let enc_sec = encrypt_rsa_pkcs1v15(keys.public_key_der(), &secret).unwrap();
        let enc_tok = encrypt_rsa_pkcs1v15(keys.public_key_der(), &nonce).unwrap();
        let recovered = keys
            .verify_encryption_response(&nonce, &enc_sec, &enc_tok)
            .unwrap();
        assert_eq!(recovered, secret);
    }

    #[test]
    fn velocity_hmac_and_bungeeguard_forwarding_reject_tampering() {
        let secret = b"super-secret-velocity-key";
        let ip: IpAddr = "203.0.113.42".parse().unwrap();
        let profile = GameProfile {
            uuid: Uuid::new_v4(),
            username: "ProxyHero".to_owned(),
            properties: vec![ProfileProperty {
                name: "textures".to_owned(),
                value: "skin_b64".to_owned(),
                signature: Some("sig_b64".to_owned()),
            }],
        };
        let mut payload = encode_velocity_player_info(secret, ip, &profile).unwrap();
        let verified = verify_velocity_player_info(secret, &payload).unwrap();
        assert_eq!(verified.client_ip, ip);
        assert_eq!(verified.profile, profile);

        // Tamper 1 bit -> rejected!
        payload[10] ^= 0x01;
        assert!(verify_velocity_player_info(secret, &payload).is_err());

        // BungeeGuard valid vs invalid token
        let mut allowed = BTreeSet::new();
        allowed.insert("guard-tok-123".to_owned());
        let raw_ok = format!(
            "mc.example.com\0198.51.100.7\0{}\0[{{\"name\":\"bungeeguard-token\",\"value\":\"guard-tok-123\"}},{{\"name\":\"textures\",\"value\":\"tex\"}}]",
            profile.uuid.simple()
        );
        let (host, bip, buuid, bprops) = parse_bungee_handshake(&raw_ok, Some(&allowed)).unwrap();
        assert_eq!(host, "mc.example.com");
        assert_eq!(bip, "198.51.100.7".parse::<IpAddr>().unwrap());
        assert_eq!(buuid, profile.uuid);
        assert_eq!(bprops.len(), 1); // bungeeguard-token stripped!

        let raw_bad = format!(
            "mc.example.com\0198.51.100.7\0{}\0[{{\"name\":\"bungeeguard-token\",\"value\":\"wrong\"}}]",
            profile.uuid.simple()
        );
        assert!(parse_bungee_handshake(&raw_bad, Some(&allowed)).is_err());

        // Token bucket rate limiter
        let t0 = Instant::now();
        let mut limiter = PacketRateLimiter::new(10, 3);
        assert!(limiter.try_acquire(t0));
        assert!(limiter.try_acquire(t0));
        assert!(limiter.try_acquire(t0));
        assert!(!limiter.try_acquire(t0));
        assert!(limiter.try_acquire(t0 + Duration::from_millis(200)));
    }
}
