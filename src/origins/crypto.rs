//! Compatibility decoder for the four BobbaXtra session streams.
//!
//! This accepts the *shared secret* exported by the client for an owned session,
//! not either participant's public Diffie-Hellman value. Packet sequence numbers
//! are supplied explicitly because each direction and stream has its own counter.
//! The legacy format has no authentication tag in the native routines reviewed.

use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
use chacha20::cipher::{KeyIvInit, StreamCipher};
use chacha20::ChaCha20;
use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::{Zeroize, Zeroizing};

use crate::model::Direction;

const SHARED_SECRET_BYTES: usize = 56;
const HKDF_SALT: &[u8] = b"BobbaXtraHKDFSalt";
// RustCrypto reserves the final IETF counter value to detect exhaustion.
const MAX_PACKET_BYTES: u64 = u32::MAX as u64 * 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StreamKind {
    Payload,
    Header,
}

/// Four derived keys; intentionally does not implement Debug or Serialize.
pub struct OriginsSession {
    client_payload: StreamKey,
    client_header: StreamKey,
    server_payload: StreamKey,
    server_header: StreamKey,
}

struct StreamKey {
    key: [u8; 32],
    base_nonce: [u8; 12],
}

impl OriginsSession {
    /// Import exactly 56 bytes of hexadecimal shared-secret material.
    /// Leading and trailing whitespace from a clipboard paste is permitted.
    pub fn from_shared_key_hex(value: &str) -> Result<Self, String> {
        let value = value.trim();
        if value.len() != SHARED_SECRET_BYTES * 2 {
            return Err("The session shared key must contain exactly 112 hexadecimal characters (56 bytes).".into());
        }
        let secret = Zeroizing::new(hex::decode(value).map_err(|_| {
            "The session shared key contains invalid hexadecimal characters.".to_string()
        })?);
        let hkdf = Hkdf::<Sha256>::new(Some(HKDF_SALT), &secret);
        Ok(Self {
            client_payload: StreamKey::derive(&hkdf, b"BobbaXtra|bobba-c2s-data")?,
            client_header: StreamKey::derive(&hkdf, b"BobbaXtra|bobba-c2s-header")?,
            server_payload: StreamKey::derive(&hkdf, b"BobbaXtra|bobba-s2c-data")?,
            server_header: StreamKey::derive(&hkdf, b"BobbaXtra|bobba-s2c-header")?,
        })
    }

    /// Encrypt or decrypt bytes using an explicit per-stream packet sequence.
    ///
    /// Both operations are the same ChaCha20 XOR operation. A successful call
    /// establishes neither packet authenticity nor correctness of the supplied
    /// secret, direction, stream, or sequence. Empty input returns empty output.
    pub fn transform(
        &self,
        direction: Direction,
        kind: StreamKind,
        sequence: u64,
        data: &[u8],
    ) -> Result<Vec<u8>, String> {
        let stream = match (direction, kind) {
            (Direction::ClientToServer, StreamKind::Payload) => &self.client_payload,
            (Direction::ClientToServer, StreamKind::Header) => &self.client_header,
            (Direction::ServerToClient, StreamKind::Payload) => &self.server_payload,
            (Direction::ServerToClient, StreamKind::Header) => &self.server_header,
        };
        stream.transform(sequence, data)
    }
}

impl StreamKey {
    fn derive(hkdf: &Hkdf<Sha256>, info: &[u8]) -> Result<Self, String> {
        let mut material = Zeroizing::new([0u8; 44]);
        hkdf.expand(info, &mut material[..])
            .map_err(|_| "Could not derive the session stream key.".to_string())?;
        let mut key = [0u8; 32];
        let mut base_nonce = [0u8; 12];
        key.copy_from_slice(&material[..32]);
        base_nonce.copy_from_slice(&material[32..]);
        Ok(Self { key, base_nonce })
    }

    fn packet_nonce(&self, sequence: u64) -> [u8; 12] {
        let mut nonce = self.base_nonce;
        let mut suffix = [0u8; 8];
        suffix.copy_from_slice(&nonce[4..]);
        let counter = u64::from_le_bytes(suffix).wrapping_add(sequence);
        nonce[4..].copy_from_slice(&counter.to_le_bytes());
        nonce
    }

    fn transform(&self, sequence: u64, data: &[u8]) -> Result<Vec<u8>, String> {
        validate_packet_len(data.len() as u64)?;
        if data.is_empty() {
            return Ok(Vec::new());
        }
        let nonce = self.packet_nonce(sequence);
        // A fresh cipher starts at block counter zero for every packet.
        let mut cipher = ChaCha20::new((&self.key).into(), (&nonce).into());
        let mut output = data.to_vec();
        cipher
            .try_apply_keystream(&mut output)
            .map_err(|_| "The packet exceeds the ChaCha20 counter capacity.".to_string())?;
        Ok(output)
    }
}

impl Drop for StreamKey {
    fn drop(&mut self) {
        self.key.zeroize();
        self.base_nonce.zeroize();
    }
}

fn validate_packet_len(len: u64) -> Result<(), String> {
    if len > MAX_PACKET_BYTES {
        Err("The packet exceeds the ChaCha20 counter capacity.".into())
    } else {
        Ok(())
    }
}

/// Decode the native standard Base64 alphabet without `=` padding.
///
/// Unlike the native helper's permissive handling of invalid characters, this
/// accepts only a canonical wire value. Whitespace and padded input are errors.
pub fn decode_wire(value: &str) -> Result<Vec<u8>, String> {
    STANDARD_NO_PAD.decode(value).map_err(|_| {
        "The wire value must be canonical standard Base64 without padding or whitespace."
            .to_string()
    })
}

/// Encode bytes in the unpadded standard Base64 format used by BobbaXtra.
pub fn encode_wire(value: &[u8]) -> String {
    STANDARD_NO_PAD.encode(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> OriginsSession {
        OriginsSession::from_shared_key_hex(&hex::encode((0u8..56).collect::<Vec<_>>())).unwrap()
    }

    #[test]
    fn rfc5869_sha256_extract_and_expand() {
        // RFC 5869 appendix A.1, also shipped with hkdf's upstream tests.
        let ikm = [0x0b; 22];
        let salt = hex::decode("000102030405060708090a0b0c").unwrap();
        let info = hex::decode("f0f1f2f3f4f5f6f7f8f9").unwrap();
        let (prk, hkdf) = Hkdf::<Sha256>::extract(Some(&salt), &ikm);
        assert_eq!(
            hex::encode(prk),
            "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"
        );
        let mut output = [0; 42];
        hkdf.expand(&info, &mut output).unwrap();
        assert_eq!(
            hex::encode(output),
            concat!(
                "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf",
                "34007208d5b887185865"
            )
        );
    }

    #[test]
    fn rfc8439_zero_key_nonce_and_counter() {
        // RFC 8439 appendix A.1, test vector 1 (block counter zero).
        let stream = StreamKey {
            key: [0; 32],
            base_nonce: [0; 12],
        };
        assert_eq!(
            hex::encode(stream.transform(0, &[0; 64]).unwrap()),
            concat!(
                "76b8e0ada0f13d90405d6ae55386bd28bdd219b8a08ded1aa836efcc8b770dc7",
                "da41597c5157488d7724e03fb8d84a376a43b8f41518a11cc387b669b2ee6586"
            )
        );
    }

    #[test]
    fn frozen_static_compatibility_fixture() {
        // Independently computed with Python hashlib/hmac and a scalar ChaCha20
        // implementation, checked against the RFC zero-counter test above.
        // Input shared-secret bytes are 00..37; packet bytes are 00..4f.
        // This is a reproducible static fixture, not a captured live session.
        let session = session();
        let data: Vec<u8> = (0..80).collect();
        let fixtures = [
            (
                Direction::ClientToServer,
                StreamKind::Payload,
                &session.client_payload,
                "025289dc6625f805ad2d61d5721b523e39f9560c8645ac7bf3584dc2780288de4b41a7f42bd97114f1e9c2fc",
                concat!(
                    "dd48e39ef308f68a6b78c5974f982db1de93007f8a5656492f00a9addbab8b37",
                    "45737d34c86f3ded7fd88df0fce4762b686fb9f9c2c86631f05305cc2af923cc",
                    "4d1d817eb20311b339ce7689f1a2341b"
                ),
            ),
            (
                Direction::ClientToServer,
                StreamKind::Header,
                &session.client_header,
                "1cac275c241e12a5aa1d90e24e78a448360afa9d562ab5628e4bf3ea8516c7e808b078d4bdd22d5265cd3485",
                concat!(
                    "06ee51a7e045528acdee513d7afc1285d57fae2d141cf5d2289a18f89ee70e3c",
                    "ecf3a9c638534e044fac5d5098feecad5d69e635f7bab39363d4fb32143af074",
                    "8c88ad0a9b8e50f759f091b9e0bf7476"
                ),
            ),
            (
                Direction::ServerToClient,
                StreamKind::Payload,
                &session.server_payload,
                "a2a651048c613ff666c929bc5d615eba30cd311a84a65141b60be064cab8c746b5824f7d9192a4abfadc9c1c",
                concat!(
                    "aee6dc705d854dc149c0f7b1dc22f83489fe8fec27cec0576fe81a2eca69dcf6",
                    "e2e187374449c7667c57459dbaec5ac8198978f83e0352626ea1d5531f2495f4",
                    "0931cee4723b4c66b53026ef3bdaaa8a"
                ),
            ),
            (
                Direction::ServerToClient,
                StreamKind::Header,
                &session.server_header,
                "bd95a16c2ab9bfd2b354ebddd540e26e054e319041b6022d1e90414603ef36c60a82e169e8d7f152d4d4c7de",
                concat!(
                    "7723f39a6069893568fbe452ec3e774d033ca392a64a29f9425c55ebf6c15e3d",
                    "ae5baaeaa5c3b39328c77155e1073d63da8f02703e58951c2d97883bebe7ab73",
                    "8b11d4e9146f7c7e77626de44ce64a06"
                ),
            ),
        ];
        for (direction, kind, stream, material, ciphertext) in fixtures {
            assert_eq!(
                format!(
                    "{}{}",
                    hex::encode(stream.key),
                    hex::encode(stream.base_nonce)
                ),
                material
            );
            assert_eq!(
                hex::encode(session.transform(direction, kind, 42, &data).unwrap()),
                ciphertext
            );
        }
    }

    #[test]
    fn streams_and_sequences_are_independent() {
        let session = session();
        let mut outputs = std::collections::HashSet::new();
        for direction in [Direction::ClientToServer, Direction::ServerToClient] {
            for stream in [StreamKind::Payload, StreamKind::Header] {
                for sequence in [0, 1, u64::MAX] {
                    let output = session
                        .transform(direction, stream, sequence, &[0; 80])
                        .unwrap();
                    assert!(outputs.insert(output.clone()));
                    // Explicit sequence calls are deterministic and reversible.
                    assert_eq!(
                        session
                            .transform(direction, stream, sequence, &[0; 80])
                            .unwrap(),
                        output
                    );
                    assert_eq!(
                        session
                            .transform(direction, stream, sequence, &output)
                            .unwrap(),
                        vec![0; 80]
                    );
                }
            }
        }
    }

    #[test]
    fn nonce_addition_is_little_endian_and_wraps_only_the_suffix() {
        let stream = StreamKey {
            key: [0; 32],
            base_nonce: [
                0x12, 0x34, 0x56, 0x78, 0xfe, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
            ],
        };
        assert_eq!(
            stream.packet_nonce(1),
            [0x12, 0x34, 0x56, 0x78, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]
        );
        assert_eq!(
            stream.packet_nonce(2),
            [0x12, 0x34, 0x56, 0x78, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            stream.packet_nonce(3),
            [0x12, 0x34, 0x56, 0x78, 1, 0, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn empty_input_has_no_stateful_side_effect() {
        let session = session();
        assert!(session
            .transform(Direction::ClientToServer, StreamKind::Payload, 0, &[])
            .unwrap()
            .is_empty());
        let before = session
            .transform(Direction::ClientToServer, StreamKind::Payload, 0, &[0; 8])
            .unwrap();
        assert!(session
            .transform(Direction::ClientToServer, StreamKind::Payload, 0, &[])
            .unwrap()
            .is_empty());
        assert_eq!(
            before,
            session
                .transform(Direction::ClientToServer, StreamKind::Payload, 0, &[0; 8])
                .unwrap()
        );
    }

    #[test]
    fn shared_key_validation() {
        for invalid in [
            "".to_string(),
            "ab".repeat(55),
            "ab".repeat(57),
            "zz".repeat(56),
            format!("0x{}", "ab".repeat(56)),
        ] {
            assert!(OriginsSession::from_shared_key_hex(&invalid).is_err());
        }
        assert!(
            OriginsSession::from_shared_key_hex(&format!(" \r\n{}\t", "AB".repeat(56))).is_ok()
        );
    }

    #[test]
    fn oversized_packet_is_rejected_without_allocating_it() {
        assert!(validate_packet_len(MAX_PACKET_BYTES).is_ok());
        assert!(validate_packet_len(MAX_PACKET_BYTES + 1).is_err());
    }

    #[test]
    fn wire_codec_matches_native_unpadded_standard_base64() {
        for (bytes, wire) in [
            (&b""[..], ""),
            (&b"f"[..], "Zg"),
            (&b"fo"[..], "Zm8"),
            (&b"foo"[..], "Zm9v"),
            (&[0xfb, 0xff][..], "+/8"),
        ] {
            assert_eq!(encode_wire(bytes), wire);
            assert_eq!(decode_wire(wire).unwrap(), bytes);
        }
        let all_bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(decode_wire(&encode_wire(&all_bytes)).unwrap(), all_bytes);
    }

    #[test]
    fn malformed_and_noncanonical_wire_values_are_rejected() {
        for wire in [
            "Zg=", "Zg==", "Zm8=", "Z", "Zh", "Zm9", "-_8", "Zg\n", " Zg", "Zg ", "☃", "====",
        ] {
            assert!(
                decode_wire(wire).is_err(),
                "accepted invalid wire input: {wire}"
            );
        }
    }
}
