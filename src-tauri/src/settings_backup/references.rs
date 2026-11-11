//! References only. Never archive the contents of external files or private keys.
use super::{Archive, ExternalFile};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub(super) fn inventory(archive: &Archive) -> Vec<ExternalFile> {
    let mut references = BTreeSet::new();
    if let Some(local) = &archive.files["sync-local.json"] {
        collect_local(&mut references, local);
    }
    if let Some(config) = &archive.files["config.json"] {
        collect_config(&mut references, config);
    }
    let ovpn_paths: Vec<_> = references
        .iter()
        .filter(|(kind, _)| kind == "openvpnOvpnPath" || kind == "ovpnPath")
        .map(|(_, path)| path.clone())
        .collect();
    for path in ovpn_paths {
        if !inspect_ovpn(&path, &mut references) {
            references.insert((
                "OVPN dependencies unavailable; inspect file on source device".into(),
                path,
            ));
        }
    }
    references
        .into_iter()
        .map(|(kind, path)| ExternalFile { kind, path })
        .collect()
}
fn collect_local(references: &mut BTreeSet<(String, String)>, local: &Value) {
    for key in super::LOCAL_PATHS {
        add(references, key, local.get(key).and_then(Value::as_str));
    }
    if let Some(folder) = local
        .get("caddyFolder")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        add(
            references,
            "Caddyfile",
            Some(&Path::new(folder).join("Caddyfile").to_string_lossy()),
        );
    }
}
fn collect_config(references: &mut BTreeSet<(String, String)>, config: &Value) {
    for (group, key) in [
        ("openvpn", "executablePath"),
        ("openvpn", "ovpnPath"),
        ("boundary", "cliPath"),
    ] {
        add(
            references,
            key,
            config
                .get(group)
                .and_then(|v| v.get(key))
                .and_then(Value::as_str),
        );
    }
    if let Some(args) = config
        .pointer("/boundary/connectSshOptions")
        .and_then(Value::as_str)
    {
        let words = shell_words::split(args).unwrap_or_default();
        for pair in words.windows(2) {
            if pair[0] == "-i" {
                add(references, "SSH identity", Some(&pair[1]));
            }
            if pair[0] == "-o" {
                if let Some(path) = pair[1].strip_prefix("IdentityFile=") {
                    add(references, "SSH identity", Some(path));
                }
            }
        }
    }
    if let Some(args) = config.pointer("/openvpn/extraArgs").and_then(Value::as_str) {
        collect_ovpn_refs(references, args, Path::new(""));
    }
}

// Only inspect explicitly referenced OVPN files. Bounded, no symlink traversal, no recursion.
fn inspect_ovpn(path: &str, references: &mut BTreeSet<(String, String)>) -> bool {
    let Ok(Some(bytes)) = super::io::read_bounded(Path::new(path)) else {
        return false;
    };
    let bytes = zeroize::Zeroizing::new(bytes);
    if bytes.len() > 1024 * 1024 {
        return false;
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return false;
    };
    let base = Path::new(path).parent().unwrap_or(Path::new(""));
    for line in text.lines().take(10000) {
        collect_ovpn_refs(references, line, base);
    }
    text.lines().count() <= 10000
}

fn add(refs: &mut BTreeSet<(String, String)>, kind: &str, path: Option<&str>) {
    if let Some(path) = path.filter(|s| !s.is_empty() && *s != "[inline]") {
        refs.insert((kind.into(), path.into()));
    }
}
fn collect_ovpn_refs(refs: &mut BTreeSet<(String, String)>, input: &str, base: &Path) {
    // OpenVPN supports quoted paths; parsing does not execute commands or expand variables.
    let Ok(words) = shell_words::split(input) else {
        return;
    };
    for pair in words.windows(2) {
        let kind = pair[0].trim_start_matches("--");
        if [
            "ca",
            "cert",
            "key",
            "pkcs12",
            "tls-auth",
            "tls-crypt",
            "tls-crypt-v2",
            "dh",
            "crl-verify",
            "auth-user-pass",
        ]
        .contains(&kind)
            && pair[1] != "[inline]"
        {
            let path = PathBuf::from(&pair[1]);
            let path = if path.is_absolute() {
                path
            } else {
                base.join(path)
            };
            add(refs, kind, Some(&path.to_string_lossy()));
        }
    }
}
