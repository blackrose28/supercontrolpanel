//! Canonical CC-plane PTY session id (`ccpty:{deviceId}:{localSessionId}`).
//! See `docs/pty-id-breaking-change/plan.md`.

const PREFIX: &str = "ccpty:";
/// Per-segment cap; mirrors `CC_PTY_SEGMENT_MAX` in `cc-server/src/pty/ccPtySessionId.ts`.
pub const CC_PTY_SEGMENT_MAX: usize = 128;
/// Hard cap on a wire-provided canonical id; mirrors `CC_PTY_SESSION_ID_MAX` in cc-server.
pub const CC_PTY_SESSION_ID_MAX: usize = PREFIX.len() + CC_PTY_SEGMENT_MAX * 2 + 1;

/// Build the canonical session id used on the CC plane (WS, ingest, DB, admin APIs).
pub fn cc_pty_session_id(device_id: &str, local_session_id: &str) -> Result<String, String> {
    let device_id = device_id.trim();
    let local_session_id = local_session_id.trim();
    if device_id.is_empty() || local_session_id.is_empty() {
        return Err("deviceId and localSessionId must be non-empty".to_string());
    }
    if device_id.len() > CC_PTY_SEGMENT_MAX || local_session_id.len() > CC_PTY_SEGMENT_MAX {
        return Err(format!(
            "deviceId and localSessionId must each be <= {CC_PTY_SEGMENT_MAX} chars"
        ));
    }
    if device_id.contains(':') || local_session_id.contains(':') {
        return Err("deviceId and localSessionId must not contain ':'".to_string());
    }
    Ok(format!("{PREFIX}{device_id}:{local_session_id}"))
}

/// Split a canonical id into `(device_id, local_session_id)`. Returns `None` if malformed
/// or if the input exceeds [`CC_PTY_SESSION_ID_MAX`].
pub fn parse_cc_pty_session_id(session_id: &str) -> Option<(String, String)> {
    let s = session_id.trim();
    if s.len() > CC_PTY_SESSION_ID_MAX {
        return None;
    }
    let rest = s.strip_prefix(PREFIX)?;
    let (device_id, local_session_id) = rest.split_once(':')?;
    if device_id.is_empty()
        || local_session_id.is_empty()
        || local_session_id.contains(':')
        || device_id.len() > CC_PTY_SEGMENT_MAX
        || local_session_id.len() > CC_PTY_SEGMENT_MAX
    {
        return None;
    }
    Some((device_id.to_string(), local_session_id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cc_pty_session_id_format_and_round_trip() {
        let id = cc_pty_session_id("dev-1", "pty-local-1").expect("id");
        assert_eq!(id, "ccpty:dev-1:pty-local-1");
        assert_eq!(
            parse_cc_pty_session_id(&id),
            Some(("dev-1".into(), "pty-local-1".into()))
        );
    }

    #[test]
    fn cc_pty_session_id_trims_segments() {
        let id = cc_pty_session_id("  dev-1  ", "  pty-1  ").expect("id");
        assert_eq!(id, "ccpty:dev-1:pty-1");
    }

    #[test]
    fn cc_pty_session_id_rejects_empty() {
        assert!(cc_pty_session_id("", "x").is_err());
        assert!(cc_pty_session_id("x", "").is_err());
    }

    #[test]
    fn cc_pty_session_id_rejects_colon_in_segment() {
        assert!(cc_pty_session_id("bad:id", "x").is_err());
        assert!(cc_pty_session_id("x", "bad:id").is_err());
    }

    #[test]
    fn parse_cc_pty_session_id_rejects_wrong_prefix() {
        assert!(parse_cc_pty_session_id("other:dev:pty").is_none());
    }

    #[test]
    fn parse_cc_pty_session_id_accepts_uuid_like_segments() {
        let d = "550e8400-e29b-41d4-a716-446655440000";
        let l = "7c9e6679-7425-40de-944b-e07fc1f90ae7";
        let id = cc_pty_session_id(d, l).expect("id");
        assert_eq!(parse_cc_pty_session_id(&id), Some((d.into(), l.into())));
    }

    #[test]
    fn cc_pty_session_id_rejects_oversized_segments() {
        let huge = "x".repeat(CC_PTY_SEGMENT_MAX + 1);
        assert!(cc_pty_session_id(&huge, "loc").is_err());
        assert!(cc_pty_session_id("dev", &huge).is_err());
    }

    #[test]
    fn parse_cc_pty_session_id_rejects_oversized_input() {
        let huge_local = "x".repeat(CC_PTY_SEGMENT_MAX + 1);
        let id = format!("{PREFIX}dev:{huge_local}");
        assert!(parse_cc_pty_session_id(&id).is_none());
        let huge_total = "x".repeat(CC_PTY_SESSION_ID_MAX + 10);
        assert!(parse_cc_pty_session_id(&huge_total).is_none());
    }
}
