use serde_json::json;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tempfile::TempDir;

use scp_core::models::{KEYRING_OPENVPN_OTP_SECRET, KEYRING_OPENVPN_PASSWORD, OPENVPN_FUNCTION_ID};
use scp_core::openvpn_ops::{
    compute_openvpn_reconnect_delay_secs, detect_openvpn_executable_checked_with,
    generate_openvpn_password_with_offset, generate_totp_code_at_epoch, openvpn_locator_result,
    OpenVpnRuntimeOverrides,
};
use scp_core::profile::ProfileLocationInput;
use scp_core::runtime::ProfileRuntime;
use scp_core::secrets::MemoryKeyringBackend;
use scp_core::supervisor::CancelToken;

// RFC 6238 Appendix B 20-byte ASCII secret "12345678901234567890" in Base32.
const RFC6238_SHA1_SECRET: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";

fn make_executable_file(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn open_vpn_runtime(
    tmp: &TempDir,
    profile_name: &str,
    ovpn_cfg: serde_json::Value,
    overrides: OpenVpnRuntimeOverrides,
) -> ProfileRuntime<MemoryKeyringBackend> {
    let cfg_dir = tmp.path().join("cfg");
    let desktop = tmp.path().join("desktop");
    let pdir = cfg_dir.join("profiles").join(profile_name);
    fs::create_dir_all(&pdir).unwrap();
    let cfg = json!({
        "app": { "defaultTerminalCols": 120, "defaultTerminalRows": 36 },
        "openvpn": ovpn_cfg
    });
    fs::write(pdir.join("config.json"), cfg.to_string()).unwrap();
    let rt = ProfileRuntime::open_with_backend(
        &ProfileLocationInput {
            config_dir: Some(&cfg_dir),
            desktop_config_dir: Some(&desktop),
            profile: Some(profile_name),
            ..Default::default()
        },
        "test",
        MemoryKeyringBackend::default(),
    )
    .unwrap();
    rt.set_openvpn_overrides(overrides);
    rt
}

#[test]
fn platform_executable_discovery() {
    assert_eq!(
        openvpn_locator_result(Some(0), b"/usr/sbin/openvpn\n").unwrap(),
        Some("/usr/sbin/openvpn".to_string())
    );
    assert_eq!(
        openvpn_locator_result(
            Some(0),
            b"C:\\OpenVPN\\bin\\openvpn.exe\r\nC:\\other.exe\r\n"
        )
        .unwrap(),
        Some(r"C:\OpenVPN\bin\openvpn.exe".to_string())
    );
    assert_eq!(openvpn_locator_result(Some(1), b"").unwrap(), None);
    for (status, stdout) in [
        (Some(2), b"err".as_slice()),
        (None, b""),
        (Some(0), b"  \n"),
        (Some(0), b"\xff"),
    ] {
        assert!(openvpn_locator_result(status, stdout).is_err());
    }

    let tmp = TempDir::new().unwrap();
    let missing = tmp.path().join("no-such-bin");
    let fallback = make_executable_file(tmp.path(), "openvpn-cand", "#!/bin/sh\n");
    let fb_str = fallback.to_string_lossy().into_owned();

    assert_eq!(
        detect_openvpn_executable_checked_with(missing.to_str().unwrap(), &[]).unwrap(),
        None
    );
    assert_eq!(
        detect_openvpn_executable_checked_with(missing.to_str().unwrap(), &[&fb_str]).unwrap(),
        Some(fb_str.clone())
    );

    let locator_ok =
        make_executable_file(tmp.path(), "loc-ok", "#!/bin/sh\necho /custom/openvpn\n");
    let rt = open_vpn_runtime(
        &tmp,
        "detect",
        json!({}),
        OpenVpnRuntimeOverrides {
            locator_program: Some(locator_ok.to_string_lossy().into_owned()),
            candidate_paths: Some(vec![]),
            ..Default::default()
        },
    );
    assert_eq!(
        rt.detect_openvpn().unwrap().as_deref(),
        Some("/custom/openvpn")
    );

    let locator_bad = make_executable_file(tmp.path(), "loc-bad", "#!/bin/sh\nexit 2\n");
    rt.set_openvpn_overrides(OpenVpnRuntimeOverrides {
        locator_program: Some(locator_bad.to_string_lossy().into_owned()),
        candidate_paths: Some(vec![]),
        ..Default::default()
    });
    assert_eq!(rt.detect_openvpn().unwrap_err().code, "OPERATION_FAILED");
}

#[test]
fn totp_and_password_generation() {
    let code = generate_totp_code_at_epoch(RFC6238_SHA1_SECRET, 59).unwrap();
    assert_eq!(code, "287082");

    let combined =
        generate_openvpn_password_with_offset("MyPass!", RFC6238_SHA1_SECRET, 0).unwrap();
    assert_eq!(combined.len(), 6 + "MyPass!".len());
    assert!(combined.ends_with("MyPass!"));
    assert!(combined[..6].chars().all(|c| c.is_ascii_digit()));

    assert_eq!(
        generate_openvpn_password_with_offset("PlainPass", "   ", 0).unwrap(),
        "PlainPass"
    );
    assert!(generate_totp_code_at_epoch("not-valid-base32!?!", 59).is_err());

    assert_eq!(compute_openvpn_reconnect_delay_secs(1, 5, 60), 0);
    assert_eq!(compute_openvpn_reconnect_delay_secs(2, 5, 60), 5);
    assert_eq!(compute_openvpn_reconnect_delay_secs(3, 5, 60), 15);
    assert_eq!(compute_openvpn_reconnect_delay_secs(10, 5, 60), 60);
}

#[test]
fn vpn_elevated_ownership_cleanup() {
    let tmp = TempDir::new().unwrap();
    let ovpn = tmp.path().join("client.ovpn");
    fs::write(&ovpn, "client\nremote 127.0.0.1 1194\n").unwrap();
    let lock_path = tmp.path().join("shared-vpn.lock");
    let capture = tmp.path().join("captured-auth.txt");

    let sudo_script = make_executable_file(
        tmp.path(),
        "fake-sudo",
        "#!/usr/bin/env python3\nimport os, sys\nargs = sys.argv[1:]\nif args == ['-n', 'true']:\n    sys.exit(1 if os.environ.get('FAKE_SUDO_DENY') == '1' else 0)\nif args and args[0] == '-n':\n    args = args[1:]\nos.execvp(args[0], args)\n",
    );
    let vpn_ok = make_executable_file(
        tmp.path(),
        "fake-openvpn-ok",
        &format!(
            "#!/usr/bin/env python3\nimport os, signal, sys, time\nsignal.signal(signal.SIGTERM, lambda *_: sys.exit(0))\nsys.stdout.write('Enter Auth Username: ')\nsys.stdout.flush()\nu = sys.stdin.readline().strip()\nsys.stdout.write('Enter Auth Password: ')\nsys.stdout.flush()\np = sys.stdin.readline().strip()\nwith open({:?}, 'w') as f:\n    f.write(f'{{u}}|{{p}}|{{sys.argv[1:]}}\\n')\nif os.environ.get('FAKE_VPN_AUTH_FAIL') == '1':\n    print('AUTH: Received control message: AUTH_FAILED', flush=True)\n    sys.exit(1)\nprint('Initialization Sequence Completed', flush=True)\ntime.sleep(30)\n",
            capture.to_string_lossy()
        ),
    );

    // 1. terminateOpenVpnWhenHelperExits=true rejected before spawn.
    let rt_bad_own = open_vpn_runtime(
        &tmp,
        "bad-own",
        json!({
            "executablePath": vpn_ok,
            "ovpnPath": ovpn,
            "username": "vpn-alice",
            "terminateOpenvpnWhenHelperExits": true
        }),
        OpenVpnRuntimeOverrides {
            sudo_program: Some(sudo_script.to_string_lossy().into_owned()),
            vpn_lock_path: Some(lock_path.clone()),
            ..Default::default()
        },
    );
    let err = rt_bad_own.start_openvpn(&CancelToken::new()).unwrap_err();
    assert_eq!(err.code, "CAPABILITY_UNAVAILABLE");
    assert_eq!(err.reason, "incompatible_vpn_ownership");

    // 2. Missing password -> credentials_missing; valid config -> ready, prompts answered, clean stop.
    let rt = open_vpn_runtime(
        &tmp,
        "ok-own",
        json!({
            "executablePath": vpn_ok,
            "ovpnPath": ovpn,
            "username": "vpn-alice",
            "extraArgs": "--verb 3"
        }),
        OpenVpnRuntimeOverrides {
            sudo_program: Some(sudo_script.to_string_lossy().into_owned()),
            vpn_lock_path: Some(lock_path.clone()),
            ready_timeout: Some(Duration::from_secs(5)),
            ..Default::default()
        },
    );
    assert_eq!(
        rt.start_openvpn(&CancelToken::new()).unwrap_err().reason,
        "credentials_missing"
    );

    rt.write_secret(KEYRING_OPENVPN_PASSWORD, Some("VpnSecret!"))
        .unwrap();
    rt.write_secret(KEYRING_OPENVPN_OTP_SECRET, Some(RFC6238_SHA1_SECRET))
        .unwrap();

    // Noninteractive sudo preflight denial releases lock and fails cleanly before spawning openvpn.
    let sudo_deny = make_executable_file(tmp.path(), "sudo-deny", "#!/bin/sh\nexit 1\n");
    rt.set_openvpn_overrides(OpenVpnRuntimeOverrides {
        sudo_program: Some(sudo_deny.to_string_lossy().into_owned()),
        vpn_lock_path: Some(lock_path.clone()),
        ..Default::default()
    });
    let elev_err = rt.start_openvpn(&CancelToken::new()).unwrap_err();
    assert_eq!(elev_err.code, "CAPABILITY_UNAVAILABLE");
    assert_eq!(elev_err.reason, "elevation_interaction_required");

    // Switch back to working sudo and start OpenVPN through prompt automation.
    rt.set_openvpn_overrides(OpenVpnRuntimeOverrides {
        sudo_program: Some(sudo_script.to_string_lossy().into_owned()),
        vpn_lock_path: Some(lock_path.clone()),
        ready_timeout: Some(Duration::from_secs(5)),
        ..Default::default()
    });
    let started = rt.start_openvpn(&CancelToken::new()).unwrap();
    assert_eq!(started.function_id, OPENVPN_FUNCTION_ID);
    let st = rt.get_function_status(OPENVPN_FUNCTION_ID).unwrap();
    assert_eq!(st.state, "running");
    assert_eq!(st.message, "VPN tunnel is up");

    let recorded = fs::read_to_string(&capture).unwrap();
    let (user, pw, rest): (&str, &str, &str) = {
        let mut parts = recorded.trim().splitn(3, '|');
        (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        )
    };
    assert_eq!(user, "vpn-alice");
    assert_eq!(pw.len(), 6 + "VpnSecret!".len());
    assert!(pw[..6].chars().all(|c| c.is_ascii_digit()));
    assert!(pw.ends_with("VpnSecret!"));
    assert!(rest.contains("--verb"));

    // Duplicate start on same runtime conflicts without spawning a second session.
    let dup = rt.start_openvpn(&CancelToken::new()).unwrap_err();
    assert_eq!(dup.code, "CONFLICT");

    assert!(rt.stop_openvpn(Duration::from_secs(2)).unwrap());
    assert!(!rt.stop_openvpn(Duration::from_secs(1)).unwrap());

    let deadline = Instant::now() + Duration::from_secs(2);
    while rt
        .get_function_status(OPENVPN_FUNCTION_ID)
        .unwrap()
        .session_id
        .is_some()
    {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(15));
    }

    // Auth failure cleans up child + releases vpn.lock and redacts password.
    let vpn_auth_fail = make_executable_file(
        tmp.path(),
        "fake-openvpn-fail",
        "#!/usr/bin/env python3\nimport sys\nsys.stdout.write('Enter Auth Username: ')\nsys.stdout.flush()\nsys.stdin.readline()\nsys.stdout.write('Enter Auth Password: ')\nsys.stdout.flush()\nsys.stdin.readline()\nprint('AUTH: Received control message: AUTH_FAILED', flush=True)\nsys.exit(1)\n",
    );
    let rt_fail = open_vpn_runtime(
        &tmp,
        "auth-fail",
        json!({
            "executablePath": vpn_auth_fail,
            "ovpnPath": ovpn,
            "username": "vpn-alice"
        }),
        OpenVpnRuntimeOverrides {
            sudo_program: Some(sudo_script.to_string_lossy().into_owned()),
            vpn_lock_path: Some(lock_path.clone()),
            ready_timeout: Some(Duration::from_secs(5)),
            ..Default::default()
        },
    );
    rt_fail
        .write_secret(KEYRING_OPENVPN_PASSWORD, Some("TopSecretVpnPw!"))
        .unwrap();
    let fail_err = rt_fail.start_openvpn(&CancelToken::new()).unwrap_err();
    assert_eq!(fail_err.code, "OPERATION_FAILED");
    assert_eq!(fail_err.reason, "openvpn_auth_failed");
    assert!(!fail_err.message.contains("TopSecretVpnPw!"));
}

#[test]
fn vpn_stop_rejects_foreign_owner() {
    let tmp = TempDir::new().unwrap();
    let ovpn = tmp.path().join("client.ovpn");
    fs::write(&ovpn, "client\nremote 127.0.0.1 1194\n").unwrap();
    let lock_path = tmp.path().join("host-wide-vpn.lock");

    let sudo_script = make_executable_file(
        tmp.path(),
        "sudo-pass",
        "#!/usr/bin/env python3\nimport os, sys\nargs = sys.argv[1:]\nif args == ['-n', 'true']:\n    sys.exit(0)\nif args and args[0] == '-n':\n    args = args[1:]\nos.execvp(args[0], args)\n",
    );
    let vpn_long = make_executable_file(
        tmp.path(),
        "openvpn-long",
        "#!/usr/bin/env python3\nimport signal, sys, time\nsignal.signal(signal.SIGTERM, lambda *_: sys.exit(0))\nprint('Initialization Sequence Completed', flush=True)\ntime.sleep(30)\n",
    );
    let ovpn_cfg = json!({
        "executablePath": vpn_long,
        "ovpnPath": ovpn,
        "username": "vpn-alice"
    });
    let overrides = OpenVpnRuntimeOverrides {
        sudo_program: Some(sudo_script.to_string_lossy().into_owned()),
        vpn_lock_path: Some(lock_path.clone()),
        ready_timeout: Some(Duration::from_secs(5)),
        ..Default::default()
    };

    let rt_a = open_vpn_runtime(&tmp, "prof-a", ovpn_cfg.clone(), overrides.clone());
    rt_a.write_secret(KEYRING_OPENVPN_PASSWORD, Some("pw-a"))
        .unwrap();
    let rt_b = open_vpn_runtime(&tmp, "prof-b", ovpn_cfg, overrides);
    rt_b.write_secret(KEYRING_OPENVPN_PASSWORD, Some("pw-b"))
        .unwrap();

    let cancel = CancelToken::new();
    let started_a = rt_a.start_openvpn(&cancel).unwrap();

    let conflict_b = rt_b.start_openvpn(&cancel).unwrap_err();
    assert_eq!(conflict_b.code, "CONFLICT");
    assert_eq!(conflict_b.reason, "vpn_owner_conflict");
    assert_eq!(conflict_b.exit_code(), 5);

    // Stopping on foreign runtime returns Ok(false) and leaves rt_a alive and locked.
    assert!(!rt_b.stop_openvpn(Duration::from_secs(1)).unwrap());
    assert_eq!(
        rt_a.supervisor()
            .active_session_for_function(OPENVPN_FUNCTION_ID)
            .as_deref(),
        Some(started_a.session_id.as_str())
    );

    rt_a.shutdown(Duration::from_secs(2));
    assert_eq!(rt_a.supervisor().session_count(), 0);

    // Once rt_a releases vpn.lock on shutdown, rt_b can acquire ownership cleanly.
    let started_b = rt_b.start_openvpn(&cancel).unwrap();
    assert!(!started_b.session_id.is_empty());
    assert!(rt_b.stop_openvpn(Duration::from_secs(2)).unwrap());
}
