//! Origins' Shockwave (FUSE) message format, as far as Hablog needs it.
//! A message starts with a two-character header such as `@D`, and numbers
//! such as lengths are written in "Base64" characters (`@` to `\x7f`), 6 bits
//! each. The server ends each message with `\x01`; the client prefixes each
//! with its length in three such characters. Server messages are returned with
//! their `\x01`, so what is shown is exactly what was sent.
//!
//! Before encryption starts, these messages travel on the wire as they are.
//! Afterwards the wire carries standard Base64 ciphertext, which never
//! contains `\x01` or `@`, so it can't be mistaken for them.
use crate::model::Direction;

/// The login message: `@D`, then the username and the password.
const LOGIN: &[u8] = b"@D";
const END: u8 = 1;

/// The messages in one decrypted payload. Server payloads can hold several.
pub fn split_payload(direction: Direction, payload: &[u8]) -> Vec<&[u8]> {
    match direction {
        Direction::ServerToClient => payload
            .split_inclusive(|&byte| byte == END)
            .filter(|message| *message != [END])
            .collect(),
        Direction::ClientToServer => vec![payload],
    }
}

/// The messages in a TCP chunk sent before encryption started, or None when
/// the chunk isn't exactly a run of plain messages (it is encrypted, or a
/// message was split across chunks).
pub fn plain_messages(direction: Direction, chunk: &[u8]) -> Option<Vec<&[u8]>> {
    let messages = match direction {
        Direction::ServerToClient => {
            if chunk.last() != Some(&END) {
                return None;
            }
            split_payload(direction, chunk)
        }
        Direction::ClientToServer => {
            let mut messages = Vec::new();
            let mut rest = chunk;
            while !rest.is_empty() {
                let (length, after) = number(rest, 3)?;
                messages.push(after.get(..length)?);
                rest = &after[length..];
            }
            messages
        }
    };
    let well_formed = !messages.is_empty() && messages.iter().all(|message| id(message).is_some());
    well_formed.then_some(messages)
}

/// The numeric ID in a message's two-character header.
pub fn id(message: &[u8]) -> Option<u16> {
    number(message, 2).map(|(id, _)| id as u16)
}

fn number(bytes: &[u8], digits: usize) -> Option<(usize, &[u8])> {
    let (digits, rest) = (bytes.get(..digits)?, &bytes[digits..]);
    digits
        .iter()
        .try_fold(0usize, |value, &digit| {
            (64..128)
                .contains(&digit)
                .then(|| (value << 6) | usize::from(digit & 0x3f))
        })
        .map(|value| (value, rest))
}

/// Reads a B64 number of `digits` characters, e.g. `@J` = 10 or `CN` = 206.
pub fn b64(bytes: &[u8], digits: usize) -> Option<(usize, &[u8])> {
    number(bytes, digits)
}

/// Writes `value` as a B64 number of `digits` characters, if it fits.
pub fn b64_encode(value: usize, digits: usize) -> Option<Vec<u8>> {
    if digits == 0 || value >> (6 * digits) != 0 {
        return None;
    }
    Some(
        (0..digits)
            .rev()
            .map(|digit| 64 | ((value >> (6 * digit)) & 0x3f) as u8)
            .collect(),
    )
}

/// Reads one VL64 number from the start of `bytes`. The first character
/// holds how many characters the number uses (bits 3-5), its sign (bit 2)
/// and its lowest 2 bits; each further character adds 6 bits.
pub fn vl64(bytes: &[u8]) -> Option<(i32, &[u8])> {
    let first = *bytes.first()?;
    if !(64..128).contains(&first) {
        return None;
    }
    let count = usize::from((first >> 3) & 7);
    if !(1..=6).contains(&count) || bytes.len() < count {
        return None;
    }
    let mut value = i64::from(first & 3);
    for (index, &byte) in bytes[1..count].iter().enumerate() {
        if !(64..128).contains(&byte) {
            return None;
        }
        value |= i64::from(byte & 0x3f) << (2 + 6 * index);
    }
    let value = if first & 4 != 0 { -value } else { value };
    Some((i32::try_from(value).ok()?, &bytes[count..]))
}

/// Writes `value` as VL64, e.g. 0 = `H`, 8 = `PB`, -1 = `M`.
pub fn vl64_encode(value: i32) -> Vec<u8> {
    let mut rest = value.unsigned_abs();
    let mut bytes = vec![64 | (rest & 3) as u8];
    rest >>= 2;
    while rest != 0 {
        bytes.push(64 | (rest & 0x3f) as u8);
        rest >>= 6;
    }
    bytes[0] |= ((bytes.len() as u8) << 3) | if value < 0 { 4 } else { 0 };
    bytes
}

/// A copy of a decrypted client message with the login password replaced by
/// `*`, or None when it isn't a well-formed login message.
pub fn mask_login_password(message: &[u8]) -> Option<Vec<u8>> {
    let rest = message.strip_prefix(LOGIN)?;
    let (username, rest) = number(rest, 2)?;
    let (password, rest) = number(rest.get(username..)?, 2)?;
    let start = message.len() - rest.len();
    let end = start.checked_add(password)?;
    if end > message.len() {
        return None;
    }
    let mut masked = message.to_vec();
    masked[start..end].fill(b'*');
    Some(masked)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(value: &str) -> Vec<u8> {
        let mut bytes = vec![
            64 | (value.len() >> 6) as u8,
            64 | (value.len() & 0x3f) as u8,
        ];
        bytes.extend_from_slice(value.as_bytes());
        bytes
    }

    #[test]
    fn masks_only_the_password_of_a_login() {
        let mut login = b"@D".to_vec();
        login.extend(string("someone@example.com"));
        login.extend(string("hunter2!"));
        login.extend_from_slice(b"@@@@");
        assert_eq!(
            mask_login_password(&login).unwrap(),
            b"@D@Ssomeone@example.com@H********@@@@"
        );
    }

    #[test]
    fn handles_long_strings() {
        let mut login = b"@D".to_vec();
        login.extend(string("me"));
        login.extend(string(&"x".repeat(70)));
        let masked = mask_login_password(&login).unwrap();
        // 70 = 1 * 64 + 6, written as "AF".
        assert_eq!(&masked[..8], b"@D@BmeAF");
        assert!(masked[8..].iter().all(|&byte| byte == b'*'));
    }

    #[test]
    fn reads_the_plain_handshake_from_the_wire() {
        // Seen at the start of a real session, before encryption.
        assert_eq!(
            plain_messages(Direction::ServerToClient, b"@@\x01"),
            Some(vec![&b"@@\x01"[..]])
        );
        assert_eq!(
            plain_messages(Direction::ClientToServer, b"@@BCN"),
            Some(vec![&b"CN"[..]])
        );
        assert_eq!(id(b"CN"), Some(206));
        assert_eq!(
            plain_messages(Direction::ServerToClient, b"DU5742\x02\x01@A12\x01"),
            Some(vec![&b"DU5742\x02\x01"[..], &b"@A12\x01"[..]])
        );
        assert_eq!(
            plain_messages(Direction::ClientToServer, b"@@BCN@@D@A12"),
            Some(vec![&b"CN"[..], &b"@A12"[..]])
        );
    }

    #[test]
    fn encrypted_or_split_chunks_are_not_plain_messages() {
        for chunk in [
            &b"UWfmhAUmg"[..],
            b"Q9F9XA1E/bDz17m1",
            b"RZ2W3gtwk",
            b"@@D@A1",
        ] {
            assert_eq!(plain_messages(Direction::ClientToServer, chunk), None);
        }
        assert_eq!(
            plain_messages(
                Direction::ServerToClient,
                b"sPeknA4ye+VkEv7TJBjvDEtr4heOHeeL"
            ),
            None
        );
        assert_eq!(plain_messages(Direction::ServerToClient, b"@@hel"), None);
        assert_eq!(plain_messages(Direction::ServerToClient, b"\x01"), None);
    }

    #[test]
    fn splits_server_payloads_on_the_end_marker() {
        assert_eq!(
            split_payload(Direction::ServerToClient, b"@CIHH\x01"),
            vec![&b"@CIHH\x01"[..]]
        );
        assert_eq!(
            split_payload(Direction::ServerToClient, b"@Ba\x01@Cb\x01"),
            vec![&b"@Ba\x01"[..], &b"@Cb\x01"[..]]
        );
        // A message cut off without its end marker is still shown.
        assert_eq!(
            split_payload(Direction::ServerToClient, b"@Ba\x01\x01@Cb"),
            vec![&b"@Ba\x01"[..], &b"@Cb"[..]]
        );
        assert_eq!(
            split_payload(Direction::ClientToServer, b"@Ka\x01b"),
            vec![&b"@Ka\x01b"[..]]
        );
        assert_eq!(id(b"@Z@Jclub_habbo"), Some(26));
        assert_eq!(id(b"\x7f@"), Some(4032));
        assert_eq!(id(b"1x"), None);
    }

    #[test]
    fn vl64_matches_values_seen_in_real_sessions() {
        // From a decrypted session: the 8 rights, then the 6 session parameters.
        assert_eq!(vl64(b"PBfuse_login"), Some((8, &b"fuse_login"[..])));
        let mut rest = &b"RAHHIIKHJHPAIQAdd-MM-yyyy"[..];
        let mut values = Vec::new();
        while let Some((value, after)) = vl64(rest) {
            values.push(value);
            rest = after;
        }
        assert_eq!(values, [6, 0, 0, 1, 1, 3, 0, 2, 0, 4, 1, 5]);
        assert_eq!(rest, b"dd-MM-yyyy");
        assert_eq!(vl64(b"M"), Some((-1, &b""[..])));
        // A first character with no length, or a cut-off number.
        assert_eq!(vl64(b"@J"), None);
        assert_eq!(vl64(b"P"), None);
        assert_eq!(vl64(b"P-"), None);
    }

    #[test]
    fn vl64_round_trips() {
        assert_eq!(vl64_encode(0), b"H");
        assert_eq!(vl64_encode(8), b"PB");
        assert_eq!(vl64_encode(-1), b"M");
        for value in [
            0,
            1,
            3,
            4,
            63,
            64,
            255,
            256,
            4095,
            70_000,
            1 << 20,
            i32::MAX,
            -5,
            -70_000,
            i32::MIN + 1,
        ] {
            let encoded = vl64_encode(value);
            assert_eq!(vl64(&encoded), Some((value, &b""[..])), "{value}");
        }
    }

    #[test]
    fn b64_round_trips_and_refuses_what_does_not_fit() {
        assert_eq!(b64_encode(206, 2).unwrap(), b"CN");
        assert_eq!(b64_encode(10, 2).unwrap(), b"@J");
        assert_eq!(b64_encode(139, 3).unwrap(), b"@BK");
        assert_eq!(b64_encode(4096, 2), None);
        assert_eq!(b64(b"@BK", 3), Some((139, &b""[..])));
    }

    #[test]
    fn leaves_everything_else_alone() {
        assert_eq!(mask_login_password(b"@Z@Jclub_habbo"), None);
        assert_eq!(mask_login_password(b"@D"), None);
        // Lengths running past the end are not a login.
        assert_eq!(mask_login_password(b"@D@Bme@Zshort"), None);
        assert_eq!(mask_login_password(b"@D\x01\x02xx"), None);
    }
}
