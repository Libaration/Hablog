//! The text tools shared by the CLI and the GUI: decoding and building FUSE
//! values, and describing what discovery found.
use crate::logger::{from_readable, readable};
use crate::model::Direction;
use crate::origins::discovery::{DiscoverySnapshot, ObservedEndpoint};
use crate::origins::fuse;

/// What `value` means as FUSE numbers, one line per reading. A number is
/// encoded instead.
pub fn decode(value: &str) -> Vec<String> {
    let value = value.trim();
    if let Ok(number) = value.parse::<i32>() {
        let mut lines = vec![format!(
            "VL64: {}",
            readable(&fuse::vl64_encode(number), usize::MAX)
        )];
        let b64: Vec<String> = usize::try_from(number)
            .ok()
            .into_iter()
            .flat_map(|number| {
                [
                    (2, "a header or string length"),
                    (3, "a client message length"),
                ]
                .into_iter()
                .filter_map(move |(digits, use_)| {
                    let encoded = fuse::b64_encode(number, digits)?;
                    Some(format!("{} ({use_})", readable(&encoded, usize::MAX)))
                })
            })
            .collect();
        lines.push(if b64.is_empty() {
            "B64:  doesn't fit".into()
        } else {
            format!("B64:  {}", b64.join(", "))
        });
        return lines;
    }

    let bytes = from_readable(value);
    let mut lines = Vec::new();
    // VL64 numbers usually come one after another; reading stops at the
    // first byte that can't be part of one, such as a [2]. Lowercase letters
    // are valid VL64 too, but four or more in a row are a word, not a number.
    let mut rest = &bytes[..];
    let mut numbers = Vec::new();
    while let Some((number, after)) = fuse::vl64(rest) {
        let used = &rest[..rest.len() - after.len()];
        if used.len() >= 4 && used.iter().all(|&b| b.is_ascii_lowercase() || b == b'_') {
            break;
        }
        numbers.push(format!("{} = {number}", readable(used, usize::MAX)));
        rest = after;
    }
    lines.push(match (numbers.is_empty(), rest.is_empty()) {
        (true, _) => "VL64: not a VL64 number".into(),
        (false, true) => format!("VL64: {}", numbers.join(", ")),
        (false, false) => format!(
            "VL64: {}, then {}",
            numbers.join(", "),
            readable(rest, usize::MAX)
        ),
    });
    lines.push(match bytes.len() {
        2 | 3 => match fuse::b64(&bytes, bytes.len()) {
            Some((number, _)) => format!("B64:  {number}"),
            None => "B64:  not a B64 number".into(),
        },
        // A client string: its length in two B64 characters, then the text.
        _ => match fuse::b64(&bytes, 2).filter(|(length, after)| *length <= after.len()) {
            Some((length, after)) if bytes.len() > 3 => {
                let text = readable(&after[..length], usize::MAX);
                if after.len() == length {
                    format!("B64:  a {length}-character string: {text}")
                } else {
                    format!(
                        "B64:  a {length}-character string: {text}, then {}",
                        readable(&after[length..], usize::MAX)
                    )
                }
            }
            _ => "B64:  not a B64 number or string".into(),
        },
    });
    lines
}

pub const BUILD_HELP: &str = "\
Start with the header, as in the log (@C) or as its ID (3), then the values:
  i:5        a number (VL64)
  b:true     a boolean, true/false or 1/0 (VL64 I/H)
  s:text     a string; ends with [2] for the client, carries its length for the server
  r:text     raw bytes, exactly as written
A space followed by another i:, b:, s: or r: starts the next value, so text can
contain spaces. [n] is byte n, as in the log. A message is built as the server
sends it to the game; start with `out` for one the game sends.";

/// One typed value as written by the user: its kind (`i`, `b`, `s` or `r`)
/// and its text.
pub type Part = (char, String);

/// The message described by `spec`, written like `@C b:true i:0 s:hello`.
pub fn build(spec: &str) -> Result<Vec<u8>, String> {
    let words: Vec<&str> = spec.split(' ').collect();
    let mut index = 0;
    let missing = "The header is missing.";
    let mut direction = Direction::ServerToClient;
    let mut header = next_word(&words, &mut index).ok_or(missing)?;
    match header.to_ascii_lowercase().as_str() {
        "out" => {
            direction = Direction::ClientToServer;
            header = next_word(&words, &mut index).ok_or(missing)?;
        }
        "in" => header = next_word(&words, &mut index).ok_or(missing)?,
        _ => {}
    }
    // Values start at a word with a type; any other word continues the
    // previous value's text after a space.
    let mut parts: Vec<Part> = Vec::new();
    for &word in &words[index..] {
        match word.split_once(':') {
            Some((kind @ ("i" | "b" | "s" | "r"), text)) => {
                parts.push((kind.chars().next().unwrap_or('r'), text.to_owned()))
            }
            _ => match parts.last_mut() {
                Some(('s' | 'r', text)) => {
                    text.push(' ');
                    text.push_str(word);
                }
                _ if word.is_empty() => {}
                _ => {
                    return Err(format!(
                        "{word} has no type; write i:, b:, s: or r: before it."
                    ))
                }
            },
        }
    }
    build_parts(direction, header, &parts)
}

/// The message for a header (`@C` or `3`) and typed parts.
pub fn build_parts(direction: Direction, header: &str, parts: &[Part]) -> Result<Vec<u8>, String> {
    let header = header.trim();
    if header.is_empty() {
        return Err("The header is missing.".into());
    }
    let header =
        match header.parse::<u16>() {
            Ok(id) => id,
            Err(_) => {
                let bytes = from_readable(header);
                fuse::id(&bytes).filter(|_| bytes.len() == 2).ok_or_else(|| {
                format!("{header} is not a header; use two characters like @C, or an ID like 3.")
            })?
            }
        };
    let values = parts
        .iter()
        .map(|(kind, text)| match kind {
            'i' => text
                .trim()
                .parse()
                .map(fuse::Value::Int)
                .map_err(|_| format!("i:{text} is not a whole number.")),
            'b' => match text.trim().to_ascii_lowercase().as_str() {
                "true" | "1" => Ok(fuse::Value::Bool(true)),
                "false" | "0" => Ok(fuse::Value::Bool(false)),
                _ => Err(format!("b:{text} is not true or false.")),
            },
            's' => Ok(fuse::Value::Str(from_readable(text))),
            'r' => Ok(fuse::Value::Raw(from_readable(text))),
            other => Err(format!(
                "{other}: is not a value type; use i:, b:, s: or r:."
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    fuse::build(direction, header, &values)
}

/// The next non-empty word from `index` on.
fn next_word<'a>(words: &[&'a str], index: &mut usize) -> Option<&'a str> {
    while let Some(&word) = words.get(*index) {
        *index += 1;
        if !word.is_empty() {
            return Some(word);
        }
    }
    None
}

/// The discovered launcher, installations, endpoints and running clients.
pub fn details(snapshot: &DiscoverySnapshot) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(launcher) = &snapshot.launcher_executable {
        lines.push(format!(
            "Launcher: {} (version {})",
            launcher.display(),
            snapshot.launcher_version.as_deref().unwrap_or("unknown")
        ));
    }
    if let Some(profile) = &snapshot.profile_root {
        lines.push(format!("Launcher profile: {}", profile.display()));
    }
    if let Some(version) = &snapshot.preferred_version {
        lines.push(format!("Launcher's selected version: {version}"));
    }
    if snapshot.installations.is_empty() {
        lines.push("No Origins installation was found.".into());
    }
    for installation in &snapshot.installations {
        lines.push(format!(
            "Origins {} at {}{}",
            installation.version,
            installation.root.display(),
            if installation.registered {
                ""
            } else {
                " (not registered with the launcher)"
            }
        ));
        match &installation.current_default_executable {
            Some(_) => lines.push(format!(
                "  Habbo.exe:  game {} ({})",
                endpoint_text(installation.current_default_info_endpoint.as_ref()),
                installation
                    .current_default_server_code
                    .as_deref()
                    .unwrap_or("unknown hotel")
            )),
            None => lines.push("  Habbo.exe:  missing".into()),
        }
        for regional in &installation.regional_executables {
            lines.push(format!(
                "  {:<9} game {}, MUS {}  ({})",
                format!(
                    "{}{}",
                    regional.server_code,
                    if regional.large { " XL" } else { "" }
                ),
                endpoint_text(regional.info_endpoint.as_ref()),
                endpoint_text(regional.mus_endpoint.as_ref()),
                regional
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ));
            if let Some(site) = &regional.site_url {
                lines.push(format!("            site {}", site.value));
            }
            if let Some(variables) = &regional.external_variables_url {
                lines.push(format!(
                    "            external variables {}",
                    variables.value
                ));
            }
        }
    }
    for client in &snapshot.running_clients {
        lines.push(format!(
            "Running client: pid {} {} ({})",
            client.pid,
            client.path.display(),
            client.server_code.as_deref().unwrap_or("unknown hotel")
        ));
    }
    lines
}

pub fn endpoint_text(endpoint: Option<&ObservedEndpoint>) -> String {
    endpoint.map_or_else(
        || "unknown".into(),
        |endpoint| format!("{}:{}", endpoint.host, endpoint.port),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shown(spec: &str) -> String {
        readable(&build(spec).unwrap(), usize::MAX)
    }

    #[test]
    fn decodes_values_pasted_from_the_log() {
        assert_eq!(decode("PB"), ["VL64: PB = 8", "B64:  1026"]);
        assert_eq!(
            decode("PBfuse_login[2]fuse_trade[2]")[0],
            "VL64: PB = 8, then fuse_login[2]fuse_trade[2]"
        );
        assert_eq!(
            decode("RAHHIIKHJHPAIQAdd-MM-yyyy[2][1]")[0],
            "VL64: RA = 6, H = 0, H = 0, I = 1, I = 1, K = 3, H = 0, J = 2, H = 0, PA = 4, I = 1, QA = 5, then dd-MM-yyyy[2][1]"
        );
        assert_eq!(decode("CN"), ["VL64: not a VL64 number", "B64:  206"]);
        assert_eq!(
            decode("@Jclub_habbo")[1],
            "B64:  a 10-character string: club_habbo"
        );
    }

    #[test]
    fn builds_messages_from_typed_values() {
        assert_eq!(build("@C b:true b:false i:0").unwrap(), b"@CIHH\x01");
        assert_eq!(shown("3 s:hello world i:2"), "@Chello world[2]J[1]");
        assert_eq!(shown("in @F r:35.0"), "@F35.0[1]");
        assert_eq!(shown("out  @Z s:club_habbo"), "@Z@Jclub_habbo");
        assert_eq!(shown("@B r:a[9]b[13]"), "@Ba[9]b[13][1]");
        assert_eq!(shown("@B i:8 s:fuse_login"), "@BPBfuse_login[2][1]");
        assert_eq!(
            build_parts(
                Direction::ServerToClient,
                "DA",
                &[('i', "1".into()), ('s', "x".into())]
            )
            .unwrap(),
            b"DAIx\x02\x01"
        );
    }

    #[test]
    fn explains_what_is_wrong_with_a_message() {
        assert!(build("").unwrap_err().contains("header is missing"));
        assert!(build("out").unwrap_err().contains("header is missing"));
        assert!(build("@ i:1").unwrap_err().contains("not a header"));
        assert!(build("@C i:x").unwrap_err().contains("whole number"));
        assert!(build("@C b:2").unwrap_err().contains("true or false"));
        assert!(build("@C hello").unwrap_err().contains("no type"));
        assert!(build("5000").unwrap_err().contains("too large"));
        assert!(
            build_parts(Direction::ServerToClient, "@C", &[('x', "1".into())])
                .unwrap_err()
                .contains("not a value type")
        );
    }

    #[test]
    fn encodes_numbers() {
        assert_eq!(
            decode("206"),
            [
                "VL64: Rs",
                "B64:  CN (a header or string length), @CN (a client message length)"
            ]
        );
        assert_eq!(decode("-1"), ["VL64: M", "B64:  doesn't fit"]);
    }
}
