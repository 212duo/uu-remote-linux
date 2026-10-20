//! Compose CONTROL keyboard JSON for scripted typing. Secrets never appear in logs or Debug.
use serde_json::{json, Value};
use std::fmt;
use std::io::{BufRead, Read};
use std::time::Duration;

/// Holds secret text; Debug/Display only show length.
pub struct Secret(String);

impl Secret {
    pub fn new(value: String) -> Self {
        Self(value)
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn chars_count(&self) -> usize {
        self.0.chars().count()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret(len={})", self.0.len())
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "*** ({} bytes)", self.0.len())
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // Best-effort wipe; String capacity may retain bytes until reallocation.
        unsafe {
            let bytes = self.0.as_bytes_mut();
            for b in bytes.iter_mut() {
                *b = 0;
            }
        }
        self.0.clear();
    }
}

/// One CONTROL keyboard action (Windows VK or Mac keycode in `key`).
#[derive(Clone, Debug, PartialEq)]
pub enum KeyAction {
    Press(i32),
    Release(i32),
}

impl KeyAction {
    pub fn to_json(&self) -> Value {
        match self {
            Self::Press(k) => json!({"action":"kbd_press","key":k}),
            Self::Release(k) => json!({"action":"kbd_release","key":k}),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    pub mac: bool,
    pub delay: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mac: false,
            delay: Duration::ZERO,
        }
    }
}

pub fn parse_delay_ms(raw: Option<&str>) -> Result<Duration, String> {
    let Some(raw) = raw else {
        return Ok(Duration::ZERO);
    };
    let ms: u64 = raw
        .parse()
        .map_err(|_| String::from("delay must be a non-negative integer (milliseconds)"))?;
    if ms > 5_000 {
        return Err("delay must be at most 5000 ms".into());
    }
    Ok(Duration::from_millis(ms))
}

/// `UU_INPUT_ENV` names another variable that holds the secret; otherwise one stdin line.
pub fn load_secret(stdin: impl BufRead) -> Result<Secret, String> {
    if let Ok(name) = std::env::var("UU_INPUT_ENV") {
        validate_env_name(&name)?;
        let value = std::env::var(&name)
            .map_err(|_| "Environment variable named by UU_INPUT_ENV is not set".to_string())?;
        if value.is_empty() {
            return Err("Secret environment variable is empty".into());
        }
        if value.len() > 16 * 1024 {
            return Err("Secret exceeds 16 KiB".into());
        }
        return Ok(Secret::new(value));
    }
    let mut line = String::new();
    stdin
        .take(16 * 1024 + 1)
        .read_to_string(&mut line)
        .map_err(|_| "Cannot read secret from stdin")?;
    if line.len() > 16 * 1024 {
        return Err("Secret exceeds 16 KiB".into());
    }
    // Trim a single trailing newline only (PIN/password may end with spaces intentionally — still trim \r\n).
    if line.ends_with('\n') {
        line.pop();
        if line.ends_with('\r') {
            line.pop();
        }
    }
    if line.is_empty() {
        return Err("Secret stdin is empty (set UU_INPUT_ENV or pipe text)".into());
    }
    Ok(Secret::new(line))
}

fn validate_env_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 128 {
        return Err("UU_INPUT_ENV must be a non-empty variable name".into());
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err("UU_INPUT_ENV must be an alphanumeric variable name".into());
    }
    Ok(())
}

/// Expand template text into key actions. Brace tokens: `{Enter}`, `{Tab}`, `{Esc}`,
/// `{Backspace}`, `{Delete}`, `{Space}`, `{Ctrl+Alt+Del}`, arrows, `{Home}`, `{End}`, `{F1}`…`{F12}`.
/// Plain characters use a US QWERTY VK/Mac mapping (Shift for uppercase/symbols).
pub fn expand(text: &str, mac: bool) -> Result<Vec<KeyAction>, String> {
    let mut out = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' {
            if chars.peek() == Some(&'{') {
                chars.next();
                push_char(&mut out, '{', mac)?;
                continue;
            }
            let mut name = String::new();
            let mut closed = false;
            for n in chars.by_ref() {
                if n == '}' {
                    closed = true;
                    break;
                }
                if n == '{' || name.len() > 32 {
                    return Err("Invalid special-key token".into());
                }
                name.push(n);
            }
            if !closed {
                return Err("Unclosed special-key token".into());
            }
            push_special(&mut out, &name, mac)?;
            continue;
        }
        if c.is_control() && c != '\t' && c != '\n' && c != '\r' {
            return Err("Control characters must use special-key tokens".into());
        }
        match c {
            '\n' | '\r' => push_special(&mut out, "Enter", mac)?,
            '\t' => push_special(&mut out, "Tab", mac)?,
            _ => push_char(&mut out, c, mac)?,
        }
    }
    Ok(out)
}

fn push_special(out: &mut Vec<KeyAction>, name: &str, mac: bool) -> Result<(), String> {
    let key = match name {
        "Enter" | "Return" => Some(vk_or_mac(0x0d, 36, mac)),
        "Tab" => Some(vk_or_mac(0x09, 48, mac)),
        "Esc" | "Escape" => Some(vk_or_mac(0x1b, 53, mac)),
        "Backspace" => Some(vk_or_mac(0x08, 51, mac)),
        "Delete" | "Del" => Some(vk_or_mac(0x2e, 117, mac)),
        "Space" => Some(vk_or_mac(0x20, 49, mac)),
        "Up" => Some(vk_or_mac(0x26, 126, mac)),
        "Down" => Some(vk_or_mac(0x28, 125, mac)),
        "Left" => Some(vk_or_mac(0x25, 123, mac)),
        "Right" => Some(vk_or_mac(0x27, 124, mac)),
        "Home" => Some(vk_or_mac(0x24, 115, mac)),
        "End" => Some(vk_or_mac(0x23, 119, mac)),
        "Ctrl+Alt+Del" | "CAD" => {
            // Left Ctrl / Alt / Delete — matches Windows Secure Attention Sequence chord.
            let ctrl = vk_or_mac(0xa2, 59, mac);
            let alt = vk_or_mac(0xa4, 58, mac);
            let del = vk_or_mac(0x2e, 117, mac);
            out.push(KeyAction::Press(ctrl));
            out.push(KeyAction::Press(alt));
            out.push(KeyAction::Press(del));
            out.push(KeyAction::Release(del));
            out.push(KeyAction::Release(alt));
            out.push(KeyAction::Release(ctrl));
            return Ok(());
        }
        other if other.len() >= 2 && other.as_bytes()[0] == b'F' => {
            let n: u32 = other[1..]
                .parse()
                .map_err(|_| format!("Unknown special key {{{other}}}"))?;
            if !(1..=12).contains(&n) {
                return Err(format!("Unknown special key {{{other}}}"));
            }
            let vk = 0x70 + (n as i32 - 1);
            let mac_code = match n {
                1 => 122,
                2 => 120,
                3 => 99,
                4 => 118,
                5 => 96,
                6 => 97,
                7 => 98,
                8 => 100,
                9 => 101,
                10 => 109,
                11 => 103,
                12 => 111,
                _ => unreachable!(),
            };
            Some(vk_or_mac(vk, mac_code, mac))
        }
        other => return Err(format!("Unknown special key {{{other}}}")),
    };
    let Some(key) = key else {
        return Ok(());
    };
    out.push(KeyAction::Press(key));
    out.push(KeyAction::Release(key));
    Ok(())
}

fn vk_or_mac(vk: i32, mac: i32, is_mac: bool) -> i32 {
    if is_mac {
        mac
    } else {
        vk
    }
}

fn push_char(out: &mut Vec<KeyAction>, c: char, mac: bool) -> Result<(), String> {
    let (vk, shift) = us_qwerty_vk(c).ok_or_else(|| {
        format!("Character U+{:04X} is not on the US QWERTY mapping; use special-key tokens where needed", c as u32)
    })?;
    let key = if mac {
        // Invert: find scancode for this Windows VK via the same table viewer uses, then mac keycode.
        // For letters/digits mirror windows_vk ranges through HID scancodes used by Input::mac_keycode.
        mac_from_vk(vk).ok_or_else(|| "Character has no Mac keycode mapping".to_string())?
    } else {
        vk
    };
    let shift_key = vk_or_mac(0xa0, 56, mac); // Left Shift
    if shift {
        out.push(KeyAction::Press(shift_key));
    }
    out.push(KeyAction::Press(key));
    out.push(KeyAction::Release(key));
    if shift {
        out.push(KeyAction::Release(shift_key));
    }
    Ok(())
}

/// US QWERTY: returns (Windows VK, needs_shift).
fn us_qwerty_vk(c: char) -> Option<(i32, bool)> {
    match c {
        'a'..='z' => Some((0x41 + (c as i32 - 'a' as i32), false)),
        'A'..='Z' => Some((0x41 + (c as i32 - 'A' as i32), true)),
        '0' => Some((0x30, false)),
        '1'..='9' => Some((0x30 + (c as i32 - '0' as i32), false)),
        ' ' => Some((0x20, false)),
        '-' => Some((0xbd, false)),
        '_' => Some((0xbd, true)),
        '=' => Some((0xbb, false)),
        '+' => Some((0xbb, true)),
        '[' => Some((0xdb, false)),
        '{' => Some((0xdb, true)),
        ']' => Some((0xdd, false)),
        '}' => Some((0xdd, true)),
        '\\' => Some((0xdc, false)),
        '|' => Some((0xdc, true)),
        ';' => Some((0xba, false)),
        ':' => Some((0xba, true)),
        '\'' => Some((0xde, false)),
        '"' => Some((0xde, true)),
        '`' => Some((0xc0, false)),
        '~' => Some((0xc0, true)),
        ',' => Some((0xbc, false)),
        '<' => Some((0xbc, true)),
        '.' => Some((0xbe, false)),
        '>' => Some((0xbe, true)),
        '/' => Some((0xbf, false)),
        '?' => Some((0xbf, true)),
        '!' => Some((0x31, true)),
        '@' => Some((0x32, true)),
        '#' => Some((0x33, true)),
        '$' => Some((0x34, true)),
        '%' => Some((0x35, true)),
        '^' => Some((0x36, true)),
        '&' => Some((0x37, true)),
        '*' => Some((0x38, true)),
        '(' => Some((0x39, true)),
        ')' => Some((0x30, true)),
        _ => None,
    }
}

fn mac_from_vk(vk: i32) -> Option<i32> {
    // Match viewer::input::mac_keycode for the HID scancodes that windows_vk uses.
    let scan = match vk {
        0x41..=0x5a => 4 + (vk - 0x41),  // A-Z
        0x31..=0x39 => 30 + (vk - 0x31), // 1-9
        0x30 => 39,
        0x0d => 40,
        0x1b => 41,
        0x08 => 42,
        0x09 => 43,
        0x20 => 44,
        0xbd => 45,
        0xbb => 46,
        0xdb => 47,
        0xdd => 48,
        0xdc => 49,
        0xba => 51,
        0xde => 52,
        0xc0 => 53,
        0xbc => 54,
        0xbe => 55,
        0xbf => 56,
        0xa0 => 225, // LShift scancode → mac 56
        _ => return None,
    };
    super::input::mac_keycode(scan)
}

pub async fn send_actions(
    send: impl Fn(
        Value,
    )
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>,
    actions: &[KeyAction],
    delay: Duration,
) -> Result<(), String> {
    for (i, action) in actions.iter().enumerate() {
        send(action.to_json()).await?;
        if !delay.is_zero() && i + 1 != actions.len() {
            tokio::time::sleep(delay).await;
        }
    }
    Ok(())
}

/// Wait for CONTROL echo on an open desktop session, then send key actions.
pub async fn run_session(
    session: &mut super::session::Session,
    actions: &[KeyAction],
    delay: Duration,
) -> Result<(), String> {
    use std::sync::atomic::Ordering;
    let ready = async {
        loop {
            if session.peer.observed.control_open.load(Ordering::Acquire)
                && session.peer.observed.echo_received.load(Ordering::Acquire)
            {
                return Ok::<_, String>(());
            }
            tokio::select! {
                r = session.step() => r?,
                _ = tokio::time::sleep(Duration::from_millis(20)) => {}
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(30), ready)
        .await
        .map_err(|_| "CONTROL echo timed out")??;
    for (i, action) in actions.iter().enumerate() {
        session.peer.send_input(&action.to_json()).await?;
        if !delay.is_zero() && i + 1 != actions.len() {
            // Keep draining signaling while pacing keystrokes.
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                r = session.step() => { r?; tokio::time::sleep(delay).await; }
            }
        }
    }
    // Release nothing extra: expand() already emits balanced press/release.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn secret_debug_and_display_never_leak() {
        let s = Secret::new("hunter2".into());
        let d = format!("{s:?}");
        let p = format!("{s}");
        assert!(d.contains("len=7"));
        assert!(!d.contains("hunter"));
        assert!(!p.contains("hunter"));
        assert!(p.contains("***"));
    }

    #[test]
    fn letters_digits_and_shift_symbols_map_to_windows_vk() {
        let actions = expand("Ab1!", false).unwrap();
        // A: shift+0x41
        assert_eq!(
            &actions[0..4],
            &[
                KeyAction::Press(0xa0),
                KeyAction::Press(0x41),
                KeyAction::Release(0x41),
                KeyAction::Release(0xa0),
            ]
        );
        // b: 0x42
        assert_eq!(
            &actions[4..6],
            &[KeyAction::Press(0x42), KeyAction::Release(0x42)]
        );
        // 1: 0x31
        assert_eq!(
            &actions[6..8],
            &[KeyAction::Press(0x31), KeyAction::Release(0x31)]
        );
        // !: shift+0x31
        assert_eq!(
            &actions[8..12],
            &[
                KeyAction::Press(0xa0),
                KeyAction::Press(0x31),
                KeyAction::Release(0x31),
                KeyAction::Release(0xa0),
            ]
        );
    }

    #[test]
    fn special_keys_and_ctrl_alt_del() {
        let a = expand("{Enter}{Tab}{Esc}{Backspace}{Ctrl+Alt+Del}", false).unwrap();
        assert!(a
            .windows(2)
            .any(|w| w == [KeyAction::Press(0x0d), KeyAction::Release(0x0d)]));
        assert!(a
            .windows(2)
            .any(|w| w == [KeyAction::Press(0x09), KeyAction::Release(0x09)]));
        assert!(a
            .windows(2)
            .any(|w| w == [KeyAction::Press(0x1b), KeyAction::Release(0x1b)]));
        assert!(a
            .windows(2)
            .any(|w| w == [KeyAction::Press(0x08), KeyAction::Release(0x08)]));
        let cad: Vec<_> = a.iter().rev().take(6).rev().cloned().collect();
        assert_eq!(
            cad,
            vec![
                KeyAction::Press(0xa2),
                KeyAction::Press(0xa4),
                KeyAction::Press(0x2e),
                KeyAction::Release(0x2e),
                KeyAction::Release(0xa4),
                KeyAction::Release(0xa2),
            ]
        );
    }

    #[test]
    fn unknown_special_key_errors_without_echoing_payload() {
        let err = expand("{NotAKey}", false).unwrap_err();
        assert!(err.contains("Unknown special key"));
        assert!(!err.contains("password"));
    }

    #[test]
    fn unclosed_token_and_control_char_rejected() {
        assert!(expand("{Enter", false).is_err());
        assert!(expand("a\u{0001}b", false).is_err());
    }

    #[test]
    fn load_from_named_env_and_stdin() {
        std::env::set_var("UU_INPUT_ENV", "WORK_LAPTOP_PIN");
        std::env::set_var("WORK_LAPTOP_PIN", "pin-value");
        let s = load_secret(Cursor::new(b"")).unwrap();
        assert_eq!(s.as_str(), "pin-value");
        assert!(!format!("{s:?}").contains("pin"));
        std::env::remove_var("UU_INPUT_ENV");
        std::env::remove_var("WORK_LAPTOP_PIN");
        let s = load_secret(Cursor::new(b"from-stdin\n")).unwrap();
        assert_eq!(s.as_str(), "from-stdin");
    }

    #[test]
    fn invalid_env_name_and_empty_secret() {
        std::env::set_var("UU_INPUT_ENV", "bad-name!");
        assert!(load_secret(Cursor::new(b"")).is_err());
        std::env::set_var("UU_INPUT_ENV", "EMPTY_SECRET_TEST");
        std::env::set_var("EMPTY_SECRET_TEST", "");
        assert!(load_secret(Cursor::new(b"")).is_err());
        std::env::remove_var("UU_INPUT_ENV");
        std::env::remove_var("EMPTY_SECRET_TEST");
        assert!(load_secret(Cursor::new(b"\n")).is_err());
    }

    #[test]
    fn delay_bounds() {
        assert_eq!(parse_delay_ms(None).unwrap(), Duration::ZERO);
        assert_eq!(
            parse_delay_ms(Some("50")).unwrap(),
            Duration::from_millis(50)
        );
        assert!(parse_delay_ms(Some("-1")).is_err());
        assert!(parse_delay_ms(Some("5001")).is_err());
    }

    #[test]
    #[test]
    fn brace_escape_types_literal_brace() {
        let a = expand("{{a", false).unwrap();
        assert_eq!(&a[0..4], &[
            KeyAction::Press(0xa0), KeyAction::Press(0xdb), KeyAction::Release(0xdb), KeyAction::Release(0xa0),
        ]);
        assert_eq!(&a[4..6], &[KeyAction::Press(0x41), KeyAction::Release(0x41)]);
    }

    #[test]
    fn password_with_enter_suffix() {
        let a = expand("secret{Enter}", false).unwrap();
        assert_eq!(a.last(), Some(&KeyAction::Release(0x0d)));
        assert_eq!(a[a.len() - 2], KeyAction::Press(0x0d));
    }

    #[tokio::test]
    async fn send_actions_respects_delay_and_order() {
        use std::sync::{Arc, Mutex};
        let log = Arc::new(Mutex::new(Vec::new()));
        let recorded = log.clone();
        let actions = expand("a{Enter}", false).unwrap();
        let start = std::time::Instant::now();
        send_actions(
            |v| {
                let recorded = recorded.clone();
                Box::pin(async move {
                    recorded.lock().unwrap().push(v);
                    Ok(())
                })
            },
            &actions,
            Duration::from_millis(20),
        )
        .await
        .unwrap();
        assert!(start.elapsed() >= Duration::from_millis(50)); // 4 events → 3 inter-event delays
        let keys: Vec<_> = log
            .lock()
            .unwrap()
            .iter()
            .map(|v| {
                (
                    v["action"].as_str().unwrap().to_owned(),
                    v["key"].as_i64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                ("kbd_press".into(), 0x41),
                ("kbd_release".into(), 0x41),
                ("kbd_press".into(), 0x0d),
                ("kbd_release".into(), 0x0d),
            ]
        );
    }
}
