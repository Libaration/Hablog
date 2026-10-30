//! Origins' Shockwave (FUSE) message format, as far as Hablog needs it.
//! A message starts with a two-character header such as `@D`; strings are
//! prefixed with their length in two "Base64" characters, 6 bits each.

/// The login message: `@D`, then the username and the password.
const LOGIN: &[u8] = b"@D";

/// A copy of a decrypted client message with the login password replaced by
/// `*`, or None when it isn't a well-formed login message.
pub fn mask_login_password(message: &[u8]) -> Option<Vec<u8>> {
    let rest = message.strip_prefix(LOGIN)?;
    let (username, rest) = length(rest)?;
    let (password, rest) = length(rest.get(username..)?)?;
    let start = message.len() - rest.len();
    let end = start.checked_add(password)?;
    if end > message.len() {
        return None;
    }
    let mut masked = message.to_vec();
    masked[start..end].fill(b'*');
    Some(masked)
}

fn length(bytes: &[u8]) -> Option<(usize, &[u8])> {
    let [high, low, rest @ ..] = bytes else {
        return None;
    };
    if !(64..128).contains(high) || !(64..128).contains(low) {
        return None;
    }
    Some((
        (usize::from(high & 0x3f) << 6) | usize::from(low & 0x3f),
        rest,
    ))
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
    fn leaves_everything_else_alone() {
        assert_eq!(mask_login_password(b"@Z@Jclub_habbo"), None);
        assert_eq!(mask_login_password(b"@D"), None);
        // Lengths running past the end are not a login.
        assert_eq!(mask_login_password(b"@D@Bme@Zshort"), None);
        assert_eq!(mask_login_password(b"@D\x01\x02xx"), None);
    }
}
