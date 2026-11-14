#![allow(dead_code)]

#[cfg(unix)]
#[path = "../tests/support/ssh_fixture.rs"]
mod ssh_fixture;

#[cfg(unix)]
fn main() {
    use std::io::{self, Read, Write};
    let fx = ssh_fixture::SshdFixture::start();
    let line = serde_json::json!({
        "port": fx.port,
        "user": fx.user,
        "hostPubAlgoAndKey": fx.host_pub_algo_and_key,
        "userKey": fx.user_key,
        "knownHosts": fx.known_hosts,
        "boundaryScript": fx.boundary_script,
        "remoteSandbox": fx.remote_sandbox,
    });
    let mut out = io::stdout().lock();
    writeln!(out, "{line}").expect("write fixture descriptor");
    out.flush().expect("flush fixture descriptor");
    let mut buf = [0u8; 64];
    while matches!(io::stdin().read(&mut buf), Ok(n) if n > 0) {}
    drop(fx);
}

#[cfg(not(unix))]
fn main() {
    eprintln!("sshd_fixture_daemon is only supported on Unix targets");
    std::process::exit(1);
}
