//! TLS 1.3 Cryptographic Handshake & Protected Record Layer (RFC 8446).
//! Provides a zero-dependency, pure-Rust implementation of:

use crate::crypto::{
    hkdf_expand, hkdf_extract, Aes128Gcm, Sha256, AES128_KEY_SIZE, GCM_NONCE_SIZE, GCM_TAG_SIZE,
    SHA256_DIGEST_SIZE,
};
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

pub const TLS_VERSION_1_3: u16 = 0x0304;
pub const TLS_LEGACY_VERSION: u16 = 0x0303; // TLS 1.2 compatibility

pub const CONTENT_TYPE_CHANGE_CIPHER_SPEC: u8 = 0x14;
pub const CONTENT_TYPE_ALERT: u8 = 0x15;
pub const CONTENT_TYPE_HANDSHAKE: u8 = 0x16;
pub const CONTENT_TYPE_APPLICATION_DATA: u8 = 0x17;

pub const HANDSHAKE_CLIENT_HELLO: u8 = 1;
pub const HANDSHAKE_SERVER_HELLO: u8 = 2;

pub const CIPHER_SUITE_TLS_AES_128_GCM_SHA256: u16 = 0x1301;

pub const EXTENSION_SERVER_NAME: u16 = 0x0000;
pub const EXTENSION_SUPPORTED_GROUPS: u16 = 0x000a;
pub const EXTENSION_KEY_SHARE: u16 = 0x0033;
pub const EXTENSION_SUPPORTED_VERSIONS: u16 = 0x002b;

/// Plaintext TLS Record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsRecord {
    pub content_type: u8,
    pub legacy_version: u16,
    pub payload: Vec<u8>,
}

impl TlsRecord {
    pub fn new(content_type: u8, payload: &[u8]) -> Self {
        Self {
            content_type,
            legacy_version: TLS_LEGACY_VERSION,
            payload: payload.to_vec(),
        }
    }

    /// Serializes standard 5-byte header + payload.
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(5 + self.payload.len());
        out.push(self.content_type);
        out.extend_from_slice(&self.legacy_version.to_be_bytes());
        out.extend_from_slice(&(self.payload.len() as u16).to_be_bytes());
        out.extend_from_slice(&self.payload);
        out
    }

    /// Parses a single TLS record from wire bytes, returning `(record, bytes_consumed)`.
    pub fn parse(data: &[u8]) -> Result<(Self, usize), &'static str> {
        if data.len() < 5 {
            return Err("Buffer too short for TLS record header");
        }
        let content_type = data[0];
        let legacy_version = u16::from_be_bytes([data[1], data[2]]);
        let length = u16::from_be_bytes([data[3], data[4]]) as usize;

        if data.len() < 5 + length {
            return Err("Incomplete TLS record payload");
        }

        let payload = data[5..5 + length].to_vec();
        Ok((
            Self {
                content_type,
                legacy_version,
                payload,
            },
            5 + length,
        ))
    }
}

/// TLS 1.3 ClientHello Handshake Message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsClientHello {
    pub random: [u8; 32],
    pub session_id: Vec<u8>,
    pub cipher_suites: Vec<u16>,
    pub server_name: Option<String>,
    pub key_share: Option<Vec<u8>>,
}

impl TlsClientHello {
    /// Creates a default ClientHello offering TLS_AES_128_GCM_SHA256 and optional SNI.
    pub fn new(server_name: Option<&str>) -> Self {
        let mut random = [0u8; 32];
        // Deterministic pseudo-random bytes for testing/reproducibility
        for i in 0..32 {
            random[i] = (i * 7 + 0x42) as u8;
        }

        Self {
            random,
            session_id: Vec::new(),
            cipher_suites: alloc::vec![CIPHER_SUITE_TLS_AES_128_GCM_SHA256],
            server_name: server_name.map(String::from),
            key_share: Some(alloc::vec![1u8; 32]), // mock X25519 key share
        }
    }

    /// Serializes ClientHello with TLS 1.3 extensions.
    pub fn serialize(&self) -> Vec<u8> {
        let mut body = Vec::new();

        // 1. legacy_version (0x0303)
        body.extend_from_slice(&TLS_LEGACY_VERSION.to_be_bytes());

        // 2. random (32 bytes)
        body.extend_from_slice(&self.random);

        // 3. legacy_session_id (1-byte length prefix)
        body.push(self.session_id.len() as u8);
        body.extend_from_slice(&self.session_id);

        // 4. cipher_suites (2-byte length prefix)
        let cs_bytes_len = (self.cipher_suites.len() * 2) as u16;
        body.extend_from_slice(&cs_bytes_len.to_be_bytes());
        for cs in &self.cipher_suites {
            body.extend_from_slice(&cs.to_be_bytes());
        }

        // 5. legacy_compression_methods (1-byte length: 1, 0x00 null compression)
        body.push(1);
        body.push(0x00);

        // 6. Extensions (2-byte length prefix)
        let mut ext_bytes = Vec::new();

        // 6a. supported_versions (0x002b) -> TLS 1.3 (0x0304)
        ext_bytes.extend_from_slice(&EXTENSION_SUPPORTED_VERSIONS.to_be_bytes());
        ext_bytes.extend_from_slice(&3u16.to_be_bytes()); // ext length: 1-byte list length + 2-byte version
        ext_bytes.push(2); // 2 bytes of versions
        ext_bytes.extend_from_slice(&TLS_VERSION_1_3.to_be_bytes());

        // 6b. SNI extension
        if let Some(ref sni) = self.server_name {
            ext_bytes.extend_from_slice(&EXTENSION_SERVER_NAME.to_be_bytes());
            let sni_bytes = sni.as_bytes();
            let entry_len = 3 + sni_bytes.len() as u16; // 1-byte name_type + 2-byte name_len + name
            let ext_len = 2 + entry_len;
            ext_bytes.extend_from_slice(&ext_len.to_be_bytes());
            ext_bytes.extend_from_slice(&entry_len.to_be_bytes());
            ext_bytes.push(0); // host_name
            ext_bytes.extend_from_slice(&(sni_bytes.len() as u16).to_be_bytes());
            ext_bytes.extend_from_slice(sni_bytes);
        }

        // 6c. key_share extension
        if let Some(ref ks) = self.key_share {
            ext_bytes.extend_from_slice(&EXTENSION_KEY_SHARE.to_be_bytes());
            let client_shares_len = 4 + ks.len() as u16; // group (2) + key_len (2) + key
            let ext_len = 2 + client_shares_len;
            ext_bytes.extend_from_slice(&ext_len.to_be_bytes());
            ext_bytes.extend_from_slice(&client_shares_len.to_be_bytes());
            ext_bytes.extend_from_slice(&0x001du16.to_be_bytes()); // x25519
            ext_bytes.extend_from_slice(&(ks.len() as u16).to_be_bytes());
            ext_bytes.extend_from_slice(ks);
        }

        body.extend_from_slice(&(ext_bytes.len() as u16).to_be_bytes());
        body.extend_from_slice(&ext_bytes);

        // Frame in 4-byte Handshake header (type = 1 ClientHello, 3-byte length)
        let body_len = body.len();
        let mut msg = Vec::with_capacity(4 + body_len);
        msg.push(HANDSHAKE_CLIENT_HELLO);
        msg.push(((body_len >> 16) & 0xFF) as u8);
        msg.push(((body_len >> 8) & 0xFF) as u8);
        msg.push((body_len & 0xFF) as u8);
        msg.extend_from_slice(&body);

        msg
    }

    /// Parses a serialized ClientHello handshake message.
    pub fn parse(data: &[u8]) -> Result<Self, &'static str> {
        if data.len() < 38 {
            return Err("ClientHello buffer too short");
        }
        if data[0] != HANDSHAKE_CLIENT_HELLO {
            return Err("Expected ClientHello handshake message");
        }

        let msg_len = ((data[1] as usize) << 16) | ((data[2] as usize) << 8) | (data[3] as usize);
        if data.len() < 4 + msg_len {
            return Err("Truncated ClientHello message");
        }

        let body = &data[4..4 + msg_len];
        if body.len() < 34 {
            return Err("Malformed ClientHello body");
        }

        let mut random = [0u8; 32];
        random.copy_from_slice(&body[2..34]);

        let sid_len = body[34] as usize;
        let mut cursor = 35;
        if body.len() < cursor + sid_len + 2 {
            return Err("Malformed session ID in ClientHello");
        }
        let session_id = body[cursor..cursor + sid_len].to_vec();
        cursor += sid_len;

        let cs_len = u16::from_be_bytes([body[cursor], body[cursor + 1]]) as usize;
        cursor += 2;
        if body.len() < cursor + cs_len + 1 {
            return Err("Malformed cipher suites in ClientHello");
        }

        let mut cipher_suites = Vec::new();
        for i in (0..cs_len).step_by(2) {
            if cursor + i + 1 < body.len() {
                let cs = u16::from_be_bytes([body[cursor + i], body[cursor + i + 1]]);
                cipher_suites.push(cs);
            }
        }
        cursor += cs_len;

        let comp_len = body[cursor] as usize;
        cursor += 1 + comp_len;

        let mut server_name = None;
        let mut key_share = None;

        if cursor + 2 <= body.len() {
            let ext_total_len = u16::from_be_bytes([body[cursor], body[cursor + 1]]) as usize;
            cursor += 2;
            let ext_end = (cursor + ext_total_len).min(body.len());

            while cursor + 4 <= ext_end {
                let ext_type = u16::from_be_bytes([body[cursor], body[cursor + 1]]);
                let ext_len = u16::from_be_bytes([body[cursor + 2], body[cursor + 3]]) as usize;
                cursor += 4;

                if cursor + ext_len > ext_end {
                    break;
                }
                let ext_data = &body[cursor..cursor + ext_len];
                cursor += ext_len;

                if ext_type == EXTENSION_SERVER_NAME && ext_data.len() > 5 {
                    let name_len = u16::from_be_bytes([ext_data[3], ext_data[4]]) as usize;
                    if ext_data.len() >= 5 + name_len {
                        if let Ok(s) = core::str::from_utf8(&ext_data[5..5 + name_len]) {
                            server_name = Some(String::from(s));
                        }
                    }
                } else if ext_type == EXTENSION_KEY_SHARE && ext_data.len() >= 6 {
                    let share_len = u16::from_be_bytes([ext_data[4], ext_data[5]]) as usize;
                    if ext_data.len() >= 6 + share_len {
                        key_share = Some(ext_data[6..6 + share_len].to_vec());
                    }
                }
            }
        }

        Ok(Self {
            random,
            session_id,
            cipher_suites,
            server_name,
            key_share,
        })
    }
}

/// TLS 1.3 ServerHello Handshake Message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsServerHello {
    pub random: [u8; 32],
    pub session_id: Vec<u8>,
    pub cipher_suite: u16,
    pub key_share: Option<Vec<u8>>,
}

impl TlsServerHello {
    pub fn new(session_id: &[u8], key_share: Option<&[u8]>) -> Self {
        let mut random = [0u8; 32];
        for i in 0..32 {
            random[i] = (i * 11 + 0x2A) as u8;
        }

        Self {
            random,
            session_id: session_id.to_vec(),
            cipher_suite: CIPHER_SUITE_TLS_AES_128_GCM_SHA256,
            key_share: key_share.map(|s| s.to_vec()),
        }
    }

    /// Serializes ServerHello with TLS 1.3 supported_versions and key_share extensions.
    pub fn serialize(&self) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&TLS_LEGACY_VERSION.to_be_bytes());
        body.extend_from_slice(&self.random);

        body.push(self.session_id.len() as u8);
        body.extend_from_slice(&self.session_id);

        body.extend_from_slice(&self.cipher_suite.to_be_bytes());
        body.push(0x00); // legacy_compression_method null

        let mut ext_bytes = Vec::new();

        // 1. supported_versions: TLS 1.3 (0x0304)
        ext_bytes.extend_from_slice(&EXTENSION_SUPPORTED_VERSIONS.to_be_bytes());
        ext_bytes.extend_from_slice(&2u16.to_be_bytes());
        ext_bytes.extend_from_slice(&TLS_VERSION_1_3.to_be_bytes());

        // 2. key_share
        if let Some(ref ks) = self.key_share {
            ext_bytes.extend_from_slice(&EXTENSION_KEY_SHARE.to_be_bytes());
            let share_len = 4 + ks.len() as u16;
            ext_bytes.extend_from_slice(&share_len.to_be_bytes());
            ext_bytes.extend_from_slice(&0x001du16.to_be_bytes()); // x25519
            ext_bytes.extend_from_slice(&(ks.len() as u16).to_be_bytes());
            ext_bytes.extend_from_slice(ks);
        }

        body.extend_from_slice(&(ext_bytes.len() as u16).to_be_bytes());
        body.extend_from_slice(&ext_bytes);

        let body_len = body.len();
        let mut msg = Vec::with_capacity(4 + body_len);
        msg.push(HANDSHAKE_SERVER_HELLO);
        msg.push(((body_len >> 16) & 0xFF) as u8);
        msg.push(((body_len >> 8) & 0xFF) as u8);
        msg.push((body_len & 0xFF) as u8);
        msg.extend_from_slice(&body);

        msg
    }

    /// Parses a serialized ServerHello handshake message.
    pub fn parse(data: &[u8]) -> Result<Self, &'static str> {
        if data.len() < 38 {
            return Err("ServerHello buffer too short");
        }
        if data[0] != HANDSHAKE_SERVER_HELLO {
            return Err("Expected ServerHello handshake message");
        }

        let msg_len = ((data[1] as usize) << 16) | ((data[2] as usize) << 8) | (data[3] as usize);
        if data.len() < 4 + msg_len {
            return Err("Truncated ServerHello message");
        }

        let body = &data[4..4 + msg_len];
        let mut random = [0u8; 32];
        random.copy_from_slice(&body[2..34]);

        let sid_len = body[34] as usize;
        let mut cursor = 35;
        if body.len() < cursor + sid_len + 3 {
            return Err("Malformed ServerHello session ID");
        }
        let session_id = body[cursor..cursor + sid_len].to_vec();
        cursor += sid_len;

        let cipher_suite = u16::from_be_bytes([body[cursor], body[cursor + 1]]);
        cursor += 2; // skip cipher suite
        cursor += 1; // skip compression method

        let mut key_share = None;

        if cursor + 2 <= body.len() {
            let ext_total_len = u16::from_be_bytes([body[cursor], body[cursor + 1]]) as usize;
            cursor += 2;
            let ext_end = (cursor + ext_total_len).min(body.len());

            while cursor + 4 <= ext_end {
                let ext_type = u16::from_be_bytes([body[cursor], body[cursor + 1]]);
                let ext_len = u16::from_be_bytes([body[cursor + 2], body[cursor + 3]]) as usize;
                cursor += 4;

                if cursor + ext_len > ext_end {
                    break;
                }
                let ext_data = &body[cursor..cursor + ext_len];
                cursor += ext_len;

                if ext_type == EXTENSION_KEY_SHARE && ext_data.len() >= 4 {
                    let share_len = u16::from_be_bytes([ext_data[2], ext_data[3]]) as usize;
                    if ext_data.len() >= 4 + share_len {
                        key_share = Some(ext_data[4..4 + share_len].to_vec());
                    }
                }
            }
        }

        Ok(Self {
            random,
            session_id,
            cipher_suite,
            key_share,
        })
    }
}

/// TLS 1.3 Key Schedule (RFC 8446 Section 7.1).
pub struct TlsKeySchedule {
    pub early_secret: [u8; SHA256_DIGEST_SIZE],
    pub handshake_secret: [u8; SHA256_DIGEST_SIZE],
    pub master_secret: [u8; SHA256_DIGEST_SIZE],
    pub client_handshake_traffic_secret: [u8; SHA256_DIGEST_SIZE],
    pub server_handshake_traffic_secret: [u8; SHA256_DIGEST_SIZE],
    pub client_app_traffic_secret: [u8; SHA256_DIGEST_SIZE],
    pub server_app_traffic_secret: [u8; SHA256_DIGEST_SIZE],
}

impl TlsKeySchedule {
    /// Formats HkdfLabel and expands keys per RFC 8446 Section 7.1.
    pub fn hkdf_expand_label(
        secret: &[u8; SHA256_DIGEST_SIZE],
        label: &str,
        context: &[u8],
        out: &mut [u8],
    ) -> Result<(), &'static str> {
        let full_label = format!("tls13 {}", label);
        let fl_bytes = full_label.as_bytes();

        let mut hkdf_label = Vec::with_capacity(4 + fl_bytes.len() + context.len());
        hkdf_label.extend_from_slice(&(out.len() as u16).to_be_bytes());
        hkdf_label.push(fl_bytes.len() as u8);
        hkdf_label.extend_from_slice(fl_bytes);
        hkdf_label.push(context.len() as u8);
        hkdf_label.extend_from_slice(context);

        hkdf_expand(secret, &hkdf_label, out)
    }

    /// Derives secret: Derive-Secret(Secret, Label, Transcript-Hash) per RFC 8446.
    pub fn derive_secret(
        secret: &[u8; SHA256_DIGEST_SIZE],
        label: &str,
        transcript_hash: &[u8; SHA256_DIGEST_SIZE],
    ) -> [u8; SHA256_DIGEST_SIZE] {
        let mut out = [0u8; SHA256_DIGEST_SIZE];
        let _ = Self::hkdf_expand_label(secret, label, transcript_hash, &mut out);
        out
    }

    /// Initializes full TLS 1.3 key schedule from shared Diffie-Hellman secret and Hello transcript.
    pub fn init(shared_secret: &[u8; 32], hello_transcript_hash: &[u8; 32]) -> Self {
        // 1. Early Secret = HKDF-Extract(0, 0)
        let zeros = [0u8; SHA256_DIGEST_SIZE];
        let early_secret = hkdf_extract(&zeros, &zeros);

        // 2. Derived Secret = Derive-Secret(Early Secret, "derived", "")
        let empty_hash = Sha256::digest(b"");
        let derived_secret = Self::derive_secret(&early_secret, "derived", &empty_hash);

        // 3. Handshake Secret = HKDF-Extract(Derived Secret, SharedKey)
        let handshake_secret = hkdf_extract(&derived_secret, shared_secret);

        // 4. Client/Server Handshake Traffic Secrets
        let client_handshake_traffic_secret =
            Self::derive_secret(&handshake_secret, "c hs traffic", hello_transcript_hash);
        let server_handshake_traffic_secret =
            Self::derive_secret(&handshake_secret, "s hs traffic", hello_transcript_hash);

        // 5. Master Secret = HKDF-Extract(Derive-Secret(Handshake Secret, "derived", ""), 0)
        let derived_hs = Self::derive_secret(&handshake_secret, "derived", &empty_hash);
        let master_secret = hkdf_extract(&derived_hs, &zeros);

        // 6. Application Traffic Secrets
        let client_app_traffic_secret =
            Self::derive_secret(&master_secret, "c ap traffic", hello_transcript_hash);
        let server_app_traffic_secret =
            Self::derive_secret(&master_secret, "s ap traffic", hello_transcript_hash);

        Self {
            early_secret,
            handshake_secret,
            master_secret,
            client_handshake_traffic_secret,
            server_handshake_traffic_secret,
            client_app_traffic_secret,
            server_app_traffic_secret,
        }
    }

    /// Derives 16-byte write key and 12-byte write IV for client or server traffic.
    pub fn derive_traffic_keys(secret: &[u8; SHA256_DIGEST_SIZE]) -> TlsTrafficKeys {
        let mut key = [0u8; AES128_KEY_SIZE];
        let mut iv = [0u8; GCM_NONCE_SIZE];

        let _ = Self::hkdf_expand_label(secret, "key", &[], &mut key);
        let _ = Self::hkdf_expand_label(secret, "iv", &[], &mut iv);

        TlsTrafficKeys::new(key, iv)
    }
}

/// Protected Record Layer Traffic Keys and AEAD Encryptor/Decryptor.
pub struct TlsTrafficKeys {
    pub key: [u8; AES128_KEY_SIZE],
    pub iv: [u8; GCM_NONCE_SIZE],
    pub sequence_number: u64,
    pub aead: Aes128Gcm,
}

impl TlsTrafficKeys {
    pub fn new(key: [u8; AES128_KEY_SIZE], iv: [u8; GCM_NONCE_SIZE]) -> Self {
        let aead = Aes128Gcm::new(&key);
        Self {
            key,
            iv,
            sequence_number: 0,
            aead,
        }
    }

    /// Computes Per-Record Nonce = IV XOR sequence_number (RFC 8446 Section 5.3).
    pub fn compute_nonce(&self) -> [u8; GCM_NONCE_SIZE] {
        let mut nonce = self.iv;
        let seq_bytes = self.sequence_number.to_be_bytes();
        for i in 0..8 {
            nonce[4 + i] ^= seq_bytes[i];
        }
        nonce
    }

    /// Encrypts `plaintext` with inner `content_type` into a protected TLS 1.3 record.
    pub fn encrypt_record(&mut self, content_type: u8, plaintext: &[u8]) -> Vec<u8> {
        let nonce = self.compute_nonce();
        self.sequence_number += 1;

        // Inner plaintext: plaintext || content_type
        let mut inner_plaintext = Vec::with_capacity(plaintext.len() + 1);
        inner_plaintext.extend_from_slice(plaintext);
        inner_plaintext.push(content_type);

        // Ciphertext length includes 16-byte GCM authentication tag
        let encrypted_len = inner_plaintext.len() + GCM_TAG_SIZE;

        // AAD is the 5-byte legacy TLS record header: [0x17, 0x03, 0x03, len_hi, len_lo]
        let mut header = [0u8; 5];
        header[0] = CONTENT_TYPE_APPLICATION_DATA;
        header[1] = 0x03;
        header[2] = 0x03;
        header[3] = ((encrypted_len >> 8) & 0xFF) as u8;
        header[4] = (encrypted_len & 0xFF) as u8;

        let (ct, tag) = self.aead.encrypt(&nonce, &header, &inner_plaintext);

        let mut record = Vec::with_capacity(5 + ct.len() + tag.len());
        record.extend_from_slice(&header);
        record.extend_from_slice(&ct);
        record.extend_from_slice(&tag);
        record
    }

    /// Decrypts a protected TLS 1.3 record, verifying AEAD authentication tag and returning `(inner_content_type, plaintext)`.
    pub fn decrypt_record(&mut self, record_bytes: &[u8]) -> Result<(u8, Vec<u8>), &'static str> {
        if record_bytes.len() < 5 + GCM_TAG_SIZE {
            return Err("Record too short for protected TLS record");
        }
        if record_bytes[0] != CONTENT_TYPE_APPLICATION_DATA {
            return Err("Expected ApplicationData (0x17) record type");
        }

        let length = u16::from_be_bytes([record_bytes[3], record_bytes[4]]) as usize;
        if record_bytes.len() < 5 + length {
            return Err("Truncated protected TLS record");
        }

        let header = &record_bytes[..5];
        let payload = &record_bytes[5..5 + length];
        let ct_len = payload.len() - GCM_TAG_SIZE;
        let ciphertext = &payload[..ct_len];
        let mut tag = [0u8; GCM_TAG_SIZE];
        tag.copy_from_slice(&payload[ct_len..]);

        let nonce = self.compute_nonce();
        self.sequence_number += 1;

        let decrypted = self.aead.decrypt(&nonce, header, ciphertext, &tag)?;

        // Find actual content type by scanning backwards through any padding zeros
        let mut idx = decrypted.len();
        while idx > 0 {
            idx -= 1;
            if decrypted[idx] != 0 {
                let inner_ct = decrypted[idx];
                let pt = decrypted[..idx].to_vec();
                return Ok((inner_ct, pt));
            }
        }

        Err("No inner content type found in TLS 1.3 decrypted plaintext")
    }
}

/// Self-test verifying ClientHello and ServerHello formatting and roundtrip parsing.
pub fn verify_tls_handshake() -> bool {
    // 1. ClientHello
    let ch = TlsClientHello::new(Some("knton.local"));
    let ch_bytes = ch.serialize();
    let parsed_ch = match TlsClientHello::parse(&ch_bytes) {
        Ok(c) => c,
        Err(_) => return false,
    };
    if parsed_ch.server_name.as_deref() != Some("knton.local") {
        return false;
    }
    if parsed_ch.cipher_suites != alloc::vec![CIPHER_SUITE_TLS_AES_128_GCM_SHA256] {
        return false;
    }

    // 2. ServerHello
    let sh = TlsServerHello::new(&[0xaa, 0xbb], Some(&[2u8; 32]));
    let sh_bytes = sh.serialize();
    let parsed_sh = match TlsServerHello::parse(&sh_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };
    if parsed_sh.session_id != alloc::vec![0xaa, 0xbb] {
        return false;
    }
    if parsed_sh.cipher_suite != CIPHER_SUITE_TLS_AES_128_GCM_SHA256 {
        return false;
    }

    true
}

/// Self-test verifying TLS 1.3 key schedule derivation.
pub fn verify_tls_key_schedule() -> bool {
    let mock_shared_secret = [0x42u8; 32];
    let mock_transcript = Sha256::digest(b"ClientHelloServerHelloTranscript");

    let schedule = TlsKeySchedule::init(&mock_shared_secret, &mock_transcript);

    // Derived secrets must be non-zero
    if schedule.handshake_secret == [0u8; 32] || schedule.master_secret == [0u8; 32] {
        return false;
    }

    // Derive traffic keys
    let mut client_keys = TlsKeySchedule::derive_traffic_keys(&schedule.client_handshake_traffic_secret);
    let server_keys = TlsKeySchedule::derive_traffic_keys(&schedule.server_handshake_traffic_secret);

    if client_keys.key == [0u8; 16] || server_keys.key == [0u8; 16] {
        return false;
    }

    // Test end-to-end record encryption and decryption across parties
    let plaintext = b"GET /api/sysinfo HTTP/1.1\r\nHost: knton.local\r\n\r\n";
    let encrypted_record = client_keys.encrypt_record(CONTENT_TYPE_APPLICATION_DATA, plaintext);

    // Decrypt on receiver
    let mut receiver_keys = TlsKeySchedule::derive_traffic_keys(&schedule.client_handshake_traffic_secret);
    let (ct_type, decrypted_pt) = match receiver_keys.decrypt_record(&encrypted_record) {
        Ok(res) => res,
        Err(_) => return false,
    };

    if ct_type != CONTENT_TYPE_APPLICATION_DATA || decrypted_pt != plaintext {
        return false;
    }

    true
}
