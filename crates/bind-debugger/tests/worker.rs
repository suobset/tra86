//! End-to-end worker test against the mock backend: launch, breakpoint,
//! continue, stop, step — verifying typed updates flow up without any live
//! debugger.

use std::time::{Duration, Instant};

use bind_core::{AttachSpec, BreakpointLocation, Command, SessionId, StepKind, TargetSpec};
use bind_debugger::{MockBackend, WorkerHandle, WorkerUpdate};

/// Collect updates until `pred` is satisfied or a timeout elapses.
fn drain_until(
    handle: &WorkerHandle,
    mut pred: impl FnMut(&[WorkerUpdate]) -> bool,
) -> Vec<WorkerUpdate> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut all = Vec::new();
    loop {
        all.extend(handle.drain());
        if pred(&all) || Instant::now() > deadline {
            return all;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn full_mock_debugging_flow() {
    let handle = WorkerHandle::spawn(SessionId::new(1), Box::new(MockBackend::default()));

    handle
        .send(Command::Attach(AttachSpec::Launch(TargetSpec::program(
            "mock",
        ))))
        .unwrap();

    let updates = drain_until(&handle, |u| {
        u.iter().any(|x| matches!(x, WorkerUpdate::Snapshot(_)))
    });
    let snap = updates
        .iter()
        .filter_map(|u| match u {
            WorkerUpdate::Snapshot(s) => Some(s),
            _ => None,
        })
        .next_back()
        .expect("expected a snapshot after launch");
    assert!(snap.process.lifecycle.is_stopped());
    assert!(!snap.disassembly.is_empty());
    assert!(snap.registers.is_some());

    // Set a breakpoint at `helper` and continue to it.
    handle
        .send(Command::AddBreakpoint {
            location: BreakpointLocation::Symbol("helper".into()),
            condition: None,
        })
        .unwrap();
    handle.send(Command::Continue).unwrap();

    let updates = drain_until(&handle, |u| {
        u.iter().any(|x| {
            matches!(x, WorkerUpdate::Event(e)
                if matches!(e.event, bind_core::DebugEvent::BreakpointHit { .. }))
        })
    });
    assert!(updates.iter().any(|x| matches!(x, WorkerUpdate::Event(e)
        if matches!(e.event, bind_core::DebugEvent::BreakpointHit { .. }))));

    // Step one instruction; expect register changes to be reported.
    handle.send(Command::Step(StepKind::Instruction)).unwrap();
    let updates = drain_until(&handle, |u| {
        u.iter()
            .filter(|x| matches!(x, WorkerUpdate::Snapshot(_)))
            .count()
            >= 2
    });
    let last_snap = updates
        .iter()
        .filter_map(|u| match u {
            WorkerUpdate::Snapshot(s) => Some(s),
            _ => None,
        })
        .next_back()
        .unwrap();
    assert_eq!(
        last_snap.selected_frame().unwrap().function.as_deref(),
        Some("helper")
    );

    handle.shutdown();
}
