//! Pushing several installers to many PCs.
//!
//! Each PC gets its own worker thread that installs the apps **one after
//! another** and waits for each result before sending the next file: two MSIs
//! at once fail with error 1618, and older agents don't queue installs
//! themselves. File transfers are capped fleet-wide (a few at a time) so the
//! master never holds more than a handful of installers in memory.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use fleetview_shared::protocol::{MasterMsg, RunSpec};

use crate::agentpkg::{self, SendOutcome};
use crate::policy::AppItem;
use crate::state::{SharedState, TargetStatus};

/// Installers being transferred at the same time, fleet-wide.
const PARALLEL_TRANSFERS: usize = 4;
/// How long to wait for one install's result (the agent's own limit is 60
/// minutes; this adds slack for the transfer and the report).
const RESULT_TIMEOUT: Duration = Duration::from_secs(70 * 60);

/// Counting semaphore for the transfer limit.
struct Slots {
    free: Mutex<usize>,
    cv: Condvar,
}

impl Slots {
    fn acquire(&self) -> SlotGuard<'_> {
        let mut n = self.free.lock().unwrap();
        while *n == 0 {
            n = self.cv.wait(n).unwrap();
        }
        *n -= 1;
        SlotGuard(self)
    }
}

struct SlotGuard<'a>(&'a Slots);

impl Drop for SlotGuard<'_> {
    fn drop(&mut self) {
        *self.0.free.lock().unwrap() += 1;
        self.0.cv.notify_one();
    }
}

/// Install `apps` on every PC in `ids`. `jobs[i]` is the job for `apps[i]`
/// (with every PC as a target). Returns at once; the work runs on threads.
pub fn start(
    state: SharedState,
    apps: Vec<AppItem>,
    jobs: Vec<u64>,
    ids: Vec<String>,
    track: bool,
    new_id: fn() -> String,
) {
    let apps = Arc::new(apps);
    let jobs = Arc::new(jobs);
    let slots = Arc::new(Slots {
        free: Mutex::new(PARALLEL_TRANSFERS),
        cv: Condvar::new(),
    });
    for id in ids {
        let (state, apps, jobs, slots) = (state.clone(), apps.clone(), jobs.clone(), slots.clone());
        std::thread::spawn(move || install_on(&state, &id, &apps, &jobs, track, &slots, new_id));
    }
}

fn install_on(
    state: &SharedState,
    id: &str,
    apps: &[AppItem],
    jobs: &[u64],
    track: bool,
    slots: &Slots,
    new_id: fn() -> String,
) {
    let mut installed_any = false;
    for (app, &job) in apps.iter().zip(jobs) {
        let finish = |status: TargetStatus, note: &str| {
            if let Ok(mut s) = state.lock() {
                s.finish_target(job, id, status, note);
            }
        };
        let (online, present) = {
            let s = state.lock().unwrap();
            match s.agents.get(id) {
                Some(a) => (a.online && a.tx.is_some(), app.installed_in(&a.software)),
                None => (false, false),
            }
        };
        if !online {
            finish(TargetStatus::Skipped, "offline — not sent");
            continue;
        }
        let Some(rev) = app.revision() else {
            finish(TargetStatus::Failed, &format!("cannot read {}", app.path));
            continue;
        };
        if present {
            if track {
                if let Ok(mut s) = state.lock() {
                    s.mark_applied(id, &app.key(), &rev);
                }
            }
            finish(
                TargetStatus::Skipped,
                &format!(
                    "already installed (a program matching “{}”)",
                    app.detect.trim()
                ),
            );
            continue;
        }

        let command_id = new_id();
        {
            let mut s = state.lock().unwrap();
            s.attach_command(job, id, &command_id);
            if track {
                s.mark_on_success(&command_id, id, &app.key(), &rev);
            }
        }
        let run = RunSpec {
            command_id,
            args: app.args_vec(),
            is_msi: app.is_msi,
        };
        let outcome = {
            let _slot = slots.acquire();
            match std::fs::read(&app.path) {
                Ok(data) => agentpkg::send_file_and_wait(
                    state,
                    id,
                    new_id(),
                    app.file_name(),
                    &data,
                    Some(run),
                ),
                Err(e) => {
                    finish(TargetStatus::Failed, &format!("cannot read file: {e}"));
                    continue;
                }
            }
        };
        if outcome != SendOutcome::Delivered {
            let status = if outcome == SendOutcome::Offline {
                TargetStatus::Skipped
            } else {
                TargetStatus::Failed
            };
            finish(
                status,
                &format!("{} not delivered ({outcome:?})", app.file_name()),
            );
            continue;
        }
        installed_any = true;
        // Wait for this install to finish before sending the next one.
        let deadline = Instant::now() + RESULT_TIMEOUT;
        loop {
            std::thread::sleep(Duration::from_secs(1));
            let s = state.lock().unwrap();
            if s.target_status(job, id) != Some(TargetStatus::Pending) {
                break;
            }
            if Instant::now() >= deadline {
                drop(s);
                finish(
                    TargetStatus::Failed,
                    "no result from the agent in 70 minutes",
                );
                break;
            }
        }
    }
    // Refresh the PC's software list so the fleet view (and the package
    // checks) see what was just installed.
    if installed_any {
        if let Ok(s) = state.lock() {
            if let Some(a) = s.agents.get(id) {
                a.send(MasterMsg::RequestSoftware);
            }
        }
    }
}
