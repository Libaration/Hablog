//! Origins' Shockwave (FUSE) message format, as far as Hablog needs it.
//! A message starts with a two-character header such as `@D`, and numbers
//! such as lengths are written in "Base64" characters (`@` to `\x7f`), 6 bits
//! each. The server ends each message with `\x01`; the client prefixes each
//! with its length in three such characters.
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
            .split(|&byte| byte == END)
            .filter(|message| !message.is_empty())
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
            Some(vec![&b"@@"[..]])
        );
        assert_eq!(
            plain_messages(Direction::ClientToServer, b"@@BCN"),
            Some(vec![&b"CN"[..]])
        );
        assert_eq!(id(b"CN"), Some(206));
        assert_eq!(
            plain_messages(Direction::ServerToClient, b"DU5742\x02\x01@A12\x01"),
            Some(vec![&b"DU5742\x02"[..], &b"@A12"[..]])
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
            vec![&b"@CIHH"[..]]
        );
        assert_eq!(
            split_payload(Direction::ServerToClient, b"@Ba\x01@Cb\x01"),
            vec![&b"@Ba"[..], &b"@Cb"[..]]
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
    fn leaves_everything_else_alone() {
        assert_eq!(mask_login_password(b"@Z@Jclub_habbo"), None);
        assert_eq!(mask_login_password(b"@D"), None);
        // Lengths running past the end are not a login.
        assert_eq!(mask_login_password(b"@D@Bme@Zshort"), None);
        assert_eq!(mask_login_password(b"@D\x01\x02xx"), None);
    }
}
