//! Thread-backed "task" sessions: in-process work that shares the supervisor's session/stop/log
//! contracts with child processes (used by the headless Boundary OIDC login).
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use scp_core::supervisor::{
    AcceptOutcome, CancelToken, ProcessSupervisor, TaskLaunch, TaskOutcome,
};

fn launch(function_id: &str) -> TaskLaunch {
    TaskLaunch {
        function_id: function_id.to_string(),
        kind: "boundary-auth",
    }
}

#[test]
fn task_runs_logs_and_reports_exit_through_the_session_contracts() {
    let sup = ProcessSupervisor::new();
    let outputs = Arc::new(Mutex::new(Vec::<(String, String, bool)>::new()));
    let outs = Arc::clone(&outputs);
    let exits = Arc::new(Mutex::new(Vec::<(String, Option<i32>, String)>::new()));
    let exits2 = Arc::clone(&exits);
    let accepted_sid = Arc::new(Mutex::new(String::new()));
    let accepted2 = Arc::clone(&accepted_sid);

    let outcome = sup
        .spawn_task_with_gate(
            launch("fn-task"),
            &CancelToken::new(),
            move |sid| {
                *accepted2.lock().unwrap() = sid.to_string();
                AcceptOutcome::Accepted(())
            },
            move |sid, code, reason| {
                exits2
                    .lock()
                    .unwrap()
                    .push((sid.to_string(), code, reason.to_string()));
            },
            Some(Arc::new(move |sid: &str, data: &str, is_pty: bool| {
                outs.lock()
                    .unwrap()
                    .push((sid.to_string(), data.to_string(), is_pty));
            })),
            Box::new(|ctx| {
                ctx.log("step one");
                ctx.log("step two");
                TaskOutcome {
                    exit_code: Some(0),
                    reason: "done".to_string(),
                }
            }),
        )
        .unwrap();
    let AcceptOutcome::Accepted(sid) = outcome else {
        panic!("task must be accepted");
    };
    assert_eq!(*accepted_sid.lock().unwrap(), sid);
    assert!(sup.wait_for_session_exit(&sid, Duration::from_secs(2)));
    assert_eq!(
        exits.lock().unwrap().as_slice(),
        [(sid.clone(), Some(0), "done".to_string())]
    );
    assert_eq!(sup.get_function_logs("fn-task"), ["step one", "step two"]);
    let outputs = outputs.lock().unwrap();
    assert_eq!(outputs.len(), 2);
    assert!(outputs.iter().all(|(s, _, pty)| s == &sid && !pty));
    assert_eq!(outputs[0].1, "step one\n");
    assert_eq!(sup.session_count(), 0);
    assert_eq!(sup.active_session_for_function("fn-task"), None);
}

#[test]
fn stop_session_cancels_the_task_and_waits_for_it_to_finish() {
    let sup = ProcessSupervisor::new();
    let started = Arc::new(AtomicBool::new(false));
    let started2 = Arc::clone(&started);
    let AcceptOutcome::Accepted(sid) = sup
        .spawn_task_with_gate(
            launch("fn-stop"),
            &CancelToken::new(),
            |_| AcceptOutcome::Accepted(()),
            |_, _, _| {},
            None,
            Box::new(move |ctx| {
                started2.store(true, Ordering::SeqCst);
                while !ctx.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                TaskOutcome {
                    exit_code: Some(130),
                    reason: "cancelled".to_string(),
                }
            }),
        )
        .unwrap()
    else {
        panic!("accepted");
    };
    let deadline = Instant::now() + Duration::from_secs(2);
    while !started.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        sup.active_session_for_function("fn-stop"),
        Some(sid.clone())
    );
    let begin = Instant::now();
    assert!(sup
        .stop_function("fn-stop", Duration::from_secs(2))
        .unwrap());
    assert!(begin.elapsed() < Duration::from_secs(1));
    assert!(sup.wait_for_session_exit(&sid, Duration::from_millis(10)));
    assert!(!sup
        .stop_function("fn-stop", Duration::from_secs(1))
        .unwrap());
}

#[test]
fn accepted_task_survives_a_later_cancel_of_the_request_token() {
    // The request-scoped token only guards acceptance (HostOwned lifetime): an MCP client
    // cancelling or disconnecting after `boundary_auth_start` returned must not abort the login.
    let sup = ProcessSupervisor::new();
    let cancel = CancelToken::new();
    let reasons = Arc::new(Mutex::new(Vec::<String>::new()));
    let reasons2 = Arc::clone(&reasons);
    let AcceptOutcome::Accepted(sid) = sup
        .spawn_task_with_gate(
            launch("fn-token"),
            &cancel,
            |_| AcceptOutcome::Accepted(()),
            move |_, _, reason| reasons2.lock().unwrap().push(reason.to_string()),
            None,
            Box::new(|ctx| {
                let deadline = std::time::Instant::now() + Duration::from_millis(400);
                while std::time::Instant::now() < deadline {
                    if ctx.is_cancelled() {
                        return TaskOutcome {
                            exit_code: Some(1),
                            reason: "cancelled".to_string(),
                        };
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                TaskOutcome {
                    exit_code: Some(0),
                    reason: "completed".to_string(),
                }
            }),
        )
        .unwrap()
    else {
        panic!("accepted");
    };
    cancel.cancel();
    assert!(sup.wait_for_session_exit(&sid, Duration::from_secs(3)));
    assert!(sup.active_session_for_function("fn-token").is_none());
    assert_eq!(reasons.lock().unwrap().as_slice(), ["completed"]);
}

#[test]
fn stop_still_cancels_a_task_whose_request_token_was_cancelled() {
    let sup = ProcessSupervisor::new();
    let cancel = CancelToken::new();
    let AcceptOutcome::Accepted(sid) = sup
        .spawn_task_with_gate(
            launch("fn-token2"),
            &cancel,
            |_| AcceptOutcome::Accepted(()),
            |_, _, _| {},
            None,
            Box::new(|ctx| {
                while !ctx.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                TaskOutcome {
                    exit_code: Some(1),
                    reason: "cancelled".to_string(),
                }
            }),
        )
        .unwrap()
    else {
        panic!("accepted");
    };
    cancel.cancel();
    assert!(sup.stop_session(&sid, Duration::from_secs(1)).unwrap());
}

#[test]
fn cancel_before_accept_never_runs_the_body() {
    let sup = ProcessSupervisor::new();
    let ran = Arc::new(AtomicUsize::new(0));
    let ran2 = Arc::clone(&ran);
    let cancel = CancelToken::new();
    cancel.cancel();
    let outcome = sup
        .spawn_task_with_gate(
            launch("fn-pre"),
            &cancel,
            |_| AcceptOutcome::Accepted(()),
            |_, _, _| {},
            None,
            Box::new(move |_| {
                ran2.fetch_add(1, Ordering::SeqCst);
                TaskOutcome {
                    exit_code: Some(0),
                    reason: String::new(),
                }
            }),
        )
        .unwrap();
    assert_eq!(outcome, AcceptOutcome::CancelledBeforeAccept);

    // Gate rejection (accept returns CancelledBeforeAccept) also skips the body.
    let outcome = sup
        .spawn_task_with_gate(
            launch("fn-pre2"),
            &CancelToken::new(),
            |_| AcceptOutcome::CancelledBeforeAccept,
            |_, _, _| {},
            None,
            Box::new(move |_| unreachable!("body must not run")),
        )
        .unwrap();
    assert_eq!(outcome, AcceptOutcome::CancelledBeforeAccept);
    assert_eq!(ran.load(Ordering::SeqCst), 0);
    assert_eq!(sup.session_count(), 0);
}

#[test]
fn write_is_rejected_and_resize_is_a_noop_for_tasks() {
    let sup = ProcessSupervisor::new();
    let AcceptOutcome::Accepted(sid) = sup
        .spawn_task_with_gate(
            launch("fn-io"),
            &CancelToken::new(),
            |_| AcceptOutcome::Accepted(()),
            |_, _, _| {},
            None,
            Box::new(|ctx| {
                while !ctx.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                TaskOutcome {
                    exit_code: Some(0),
                    reason: String::new(),
                }
            }),
        )
        .unwrap()
    else {
        panic!("accepted");
    };
    let err = sup.write(&sid, "input").unwrap_err();
    assert_eq!(err.reason, "task_not_writable");
    sup.resize(&sid, 80, 24).unwrap();
    assert!(sup.stop_session(&sid, Duration::from_secs(1)).unwrap());
}

#[test]
fn shutdown_all_stops_running_tasks() {
    let sup = ProcessSupervisor::new();
    let _ = sup
        .spawn_task_with_gate(
            launch("fn-shutdown"),
            &CancelToken::new(),
            |_| AcceptOutcome::Accepted(()),
            |_, _, _| {},
            None,
            Box::new(|ctx| {
                while !ctx.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(5));
                }
                TaskOutcome {
                    exit_code: Some(0),
                    reason: String::new(),
                }
            }),
        )
        .unwrap();
    sup.shutdown_all(&["boundary-auth"], Duration::from_secs(1));
    assert_eq!(sup.session_count(), 0);
}
