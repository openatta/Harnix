//! Spawning, ending and keeping real children.

use std::io::Read;
use std::os::fd::{FromRawFd, OwnedFd};
use std::time::{Duration, Instant};

use harnix_supervise::{ChildEvent, ChildSpec, RestartPolicy, Supervisor};

/// A pipe, both ends close-on-exec: (read, write).
fn pipe() -> (std::fs::File, OwnedFd) {
    let mut fds = [0; 2];
    // SAFETY: pipe(2) fills two descriptors that nothing else owns.
    unsafe {
        assert_eq!(libc::pipe(fds.as_mut_ptr()), 0);
        for fd in fds {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
        (std::fs::File::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))
    }
}

async fn read_all(mut file: std::fs::File) -> String {
    tokio::task::spawn_blocking(move || {
        let mut text = String::new();
        file.read_to_string(&mut text).unwrap();
        text
    })
    .await
    .unwrap()
}

fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only asks whether the pid exists.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn sh(script: &str) -> ChildSpec {
    ChildSpec::new("/bin/sh").args(["-c", script])
}

/// The child has the descriptors it declared, at the numbers it declared, and no others: not the
/// ones this process holds close-on-exec, and not one it holds without, which is what a library
/// that forgot the flag leaves behind.
#[tokio::test]
async fn the_child_gets_only_the_descriptors_it_declared() {
    let (report, report_w) = pipe();
    let (_kept_r, kept_w) = pipe();
    let (_stray_r, _stray_w) = {
        let mut fds = [0; 2];
        // SAFETY: a pipe left inheritable on purpose; the test holds it until it ends.
        unsafe {
            assert_eq!(libc::pipe(fds.as_mut_ptr()), 0);
            (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))
        }
    };
    let script = "out=; n=3; while [ $n -lt 64 ]; do if [ -e /dev/fd/$n ]; then out=\"$out $n\"; fi; n=$((n+1)); done; echo $out >&3";
    let mut child = sh(script).inherit_fd(3, report_w).inherit_fd(7, kept_w).spawn().unwrap();
    let seen = read_all(report).await;
    let _ = child.wait().await;
    assert_eq!(seen.split_whitespace().collect::<Vec<_>>(), ["3", "7"], "{seen:?}");
}

/// Descriptors are placed at their numbers whatever numbers they come from here.
#[tokio::test]
async fn descriptors_land_at_their_numbers_even_when_the_sources_are_in_the_way() {
    let (a, a_w) = pipe();
    let (b, b_w) = pipe();
    use std::os::fd::AsRawFd;
    // Swap: what is at the other's number here goes to the other's number there.
    let (x, y) = (a_w.as_raw_fd(), b_w.as_raw_fd());
    let mut child = sh("echo first >&$X; echo second >&$Y")
        .envs([("X".into(), "4".into()), ("Y".into(), "3".into())])
        .inherit_fd(4, a_w)
        .inherit_fd(3, b_w)
        .spawn()
        .unwrap();
    assert_ne!(x, y);
    let _ = child.wait().await;
    assert_eq!(read_all(a).await.trim(), "first");
    assert_eq!(read_all(b).await.trim(), "second");
}

/// The child exits when its stdin closes, and `stop` waits for that rather than killing it.
#[tokio::test]
async fn the_child_exits_when_its_stdin_closes() {
    let child = ChildSpec::new("/bin/cat").spawn().unwrap();
    let started = Instant::now();
    let status = child.stop(Duration::from_secs(10)).await.unwrap();
    assert!(status.success(), "{status:?}: it was killed");
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// A child that does not leave when its stdin closes is killed after the grace.
#[tokio::test]
async fn a_child_that_ignores_stdin_is_killed_after_the_grace() {
    let child = sh("exec sleep 300").spawn().unwrap();
    let status = child.stop(Duration::from_millis(200)).await.unwrap();
    assert!(!status.success());
}

/// What a child leaves running in its session is ended with it, even once it has been killed.
#[tokio::test]
async fn what_the_child_leaves_in_its_session_is_ended_with_it() {
    let (report, report_w) = pipe();
    let child = sh("sleep 300 >/dev/null 2>&1 & echo $! >&3; exec cat")
        .inherit_fd(3, report_w)
        .own_session()
        .spawn()
        .unwrap();
    let mut line = String::new();
    let mut report = std::io::BufReader::new(report);
    std::io::BufRead::read_line(&mut report, &mut line).unwrap();
    let sleeper: i32 = line.trim().parse().unwrap();
    assert!(alive(sleeper));
    child.stop(Duration::from_secs(5)).await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while alive(sleeper) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!alive(sleeper), "{sleeper} outlived its child");
}

fn quick_policy() -> RestartPolicy {
    RestartPolicy {
        backoff: vec![Duration::from_millis(10), Duration::from_millis(20)],
        reset_after: Duration::from_millis(400),
        quick: Duration::from_millis(250),
        give_up_after: 3,
    }
}

async fn events_until_done(mut supervisor: Supervisor) -> Vec<ChildEvent> {
    let mut seen = Vec::new();
    let all = async {
        while let Some(event) = supervisor.next_event().await {
            seen.push(event);
        }
    };
    tokio::time::timeout(Duration::from_secs(20), all).await.expect("the supervisor never finished");
    seen
}

/// A child that exits at once is started three times, with the schedule's waits between, and then
/// given up on.
#[tokio::test]
async fn a_child_that_keeps_exiting_is_started_again_with_backoff_and_then_given_up_on() {
    let supervisor = Supervisor::start(|| Ok(sh("exit 3")), quick_policy());
    let seen = events_until_done(supervisor).await;
    let started = seen.iter().filter(|e| matches!(e, ChildEvent::Started { .. })).count();
    assert_eq!(started, 3, "{seen:?}");
    let waits: Vec<_> = seen
        .iter()
        .filter_map(|e| match e {
            ChildEvent::Restarting { after } => Some(*after),
            _ => None,
        })
        .collect();
    assert_eq!(waits, [Duration::from_millis(10), Duration::from_millis(20)]);
    assert!(matches!(seen.last(), Some(ChildEvent::GaveUp { .. })), "{seen:?}");
    assert!(seen.iter().any(|e| matches!(e, ChildEvent::Exited { status, .. } if status.code() == Some(3))));
}

/// A run that lasts past the quick-exit mark ends the run of quick exits: two quick exits, a
/// healthy run, two more quick exits do not add up to giving up.
#[tokio::test]
async fn a_healthy_run_ends_the_count_of_quick_exits() {
    let runs = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let counter = runs.clone();
    let mut supervisor = Supervisor::start(
        move || {
            let n = counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            // Runs 1, 2, 4 and 5 exit at once; run 3 lasts; run 6 ends the test by staying up.
            Ok(match n {
                2 => sh("sleep 0.4"),
                5 => sh("exec cat"),
                _ => sh("exit 1"),
            })
        },
        quick_policy(),
    );
    let mut started = 0;
    let all = async {
        while let Some(event) = supervisor.next_event().await {
            match event {
                ChildEvent::Started { .. } => {
                    started += 1;
                    if started == 6 {
                        return;
                    }
                }
                ChildEvent::GaveUp { reason } => panic!("gave up: {reason}"),
                _ => {}
            }
        }
        panic!("the supervisor finished early");
    };
    tokio::time::timeout(Duration::from_secs(20), all).await.expect("never reached the sixth start");
    supervisor.stop().await;
}

/// Stopping ends the child through its stdin and says so.
#[tokio::test]
async fn stopping_the_supervisor_ends_the_child() {
    let mut supervisor = Supervisor::start(|| Ok(ChildSpec::new("/bin/cat")), quick_policy());
    let Some(ChildEvent::Started { pid }) = supervisor.next_event().await else { panic!("not started") };
    supervisor.stop().await;
    let deadline = Instant::now() + Duration::from_secs(5);
    while alive(pid as i32) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(!alive(pid as i32));
}
