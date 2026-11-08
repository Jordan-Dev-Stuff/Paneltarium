//! A background thread that owns the `MonitorService` and performs display
//! changes, so a slow mode switch never blocks the event loop (and hotkeys
//! keep responding).

use std::sync::mpsc::{self, Sender};
use std::thread;

use std::time::Duration;

use pt_core::current;
use pt_core::{AppPaths, MonitorService, Topology};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Job {
    Apply(String),
    Rescue,
    Extend,
    /// Started at sign-in: apply the sign-in profile (if the setting asks
    /// for one) once its monitors have shown up.
    SignIn,
}

/// How long to wait for monitors after sign-in: 15 tries × 2 s.
const SIGN_IN_TRIES: u32 = 15;
const SIGN_IN_DELAY: Duration = Duration::from_secs(2);

/// What happened, in words for the tray menu/tooltip and the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub ok: bool,
    pub message: String,
    /// The profile that was applied, if this job applied one successfully.
    pub applied: Option<String>,
}

pub struct Worker {
    tx: Sender<Job>,
}

impl Worker {
    /// Start the worker. `done` is called (on the worker thread) after each job.
    pub fn spawn(paths: AppPaths, done: impl Fn(Outcome) + Send + 'static) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        thread::Builder::new()
            .name("display-worker".into())
            .spawn(move || {
                let service = pt_platform::backend_from_env().map(|b| MonitorService::new(b, paths));
                for job in rx {
                    let outcome = match &service {
                        Ok(svc) => run(svc, &job),
                        Err(e) => Outcome {
                            ok: false,
                            message: format!("display backend unavailable: {e}"),
                            applied: None,
                        },
                    };
                    if outcome.ok {
                        tracing::info!("{job:?}: {}", outcome.message);
                    } else {
                        tracing::error!("{job:?}: {}", outcome.message);
                    }
                    done(outcome);
                }
            })
            .expect("spawning the display worker thread");
        Worker { tx }
    }

    pub fn send(&self, job: Job) {
        // Only fails if the worker thread died, which is already logged.
        let _ = self.tx.send(job);
    }
}

/// Perform one job. Public so it can be tested without threads.
pub fn run(svc: &MonitorService, job: &Job) -> Outcome {
    let result = match job {
        Job::SignIn => return run_sign_in(svc, SIGN_IN_TRIES, SIGN_IN_DELAY),
        Job::Apply(name) => svc.apply_profile(name, false).map(|r| {
            let mut msg = format!("Applied '{}'", r.profile);
            if !r.differences.is_empty() {
                msg += &format!(" ({})", r.differences.join("; "));
            }
            (msg, Some(r.profile))
        }),
        Job::Rescue => svc.rescue().map(|t| match t {
            Topology::Clone => ("Duplicated the display on all monitors".to_string(), None),
            Topology::Extend => (
                "Extended across all monitors (duplicating wasn't possible)".to_string(),
                None,
            ),
        }),
        Job::Extend => svc
            .set_topology(Topology::Extend)
            .map(|()| ("Extended across all monitors".to_string(), None)),
    };
    match result {
        Ok((message, applied)) => Outcome {
            ok: true,
            message,
            applied,
        },
        Err(e) => Outcome {
            applied: None,
            ok: false,
            message: match job {
                Job::Apply(name) => format!("Couldn't apply '{name}': {e}"),
                Job::Rescue => format!("Couldn't duplicate or extend: {e}"),
                Job::Extend => format!("Couldn't extend: {e}"),
                Job::SignIn => format!("Sign-in: {e}"),
            },
        },
    }
}

/// Apply the sign-in profile. Monitors (especially TVs and docks) can take
/// a few seconds to appear after sign-in, so wait until every monitor the
/// profile turns on is connected, then apply — unless it's already on
/// screen (no needless flicker). After `tries`, apply with what's there.
pub fn run_sign_in(svc: &MonitorService, tries: u32, delay: Duration) -> Outcome {
    let done = |ok: bool, message: String| Outcome {
        ok,
        message,
        applied: None,
    };
    let name = match svc.startup_target() {
        Ok(Some(n)) => n,
        Ok(None) => return done(true, "Sign-in: leaving the monitors as Windows set them up".into()),
        Err(e) => return done(false, format!("Sign-in: {e}")),
    };
    for attempt in 1..=tries {
        if let Ok((profile, plan)) = svc.plan_profile(&name)
            && plan.missing.is_empty()
        {
            if let Ok(now) = svc.backend().current_layout()
                && current::matches(&profile.layout, &now)
            {
                return done(true, format!("Sign-in: '{name}' is already on screen"));
            }
            let mut o = run(svc, &Job::Apply(name));
            o.message = format!("Sign-in: {}", o.message);
            return o;
        }
        tracing::info!("sign-in: waiting for the monitors of '{name}' ({attempt}/{tries})");
        if attempt < tries {
            thread::sleep(delay);
        }
    }
    let mut o = run(svc, &Job::Apply(name));
    o.message = format!("Sign-in (not every monitor showed up): {}", o.message);
    o
}

#[cfg(test)]
mod tests {
    use super::*;
    use pt_core::mock::MockDisplayBackend;
    use pt_core::testing::sample_layout;

    #[test]
    fn jobs_report_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        let svc = MonitorService::new(
            Box::new(MockDisplayBackend::new(sample_layout())),
            AppPaths::new(dir.path()),
        );
        svc.save_current("Desk", None, false).unwrap();

        let o = run(&svc, &Job::Rescue);
        assert!(o.ok && o.message.contains("Duplicated"), "{o:?}");
        let o = run(&svc, &Job::Apply("Desk".into()));
        assert_eq!(
            o,
            Outcome {
                ok: true,
                message: "Applied 'Desk'".into(),
                applied: Some("Desk".into()),
            }
        );
        let o = run(&svc, &Job::Apply("Nope".into()));
        assert!(!o.ok && o.message.starts_with("Couldn't apply 'Nope'"), "{o:?}");
    }

    #[test]
    fn sign_in_follows_the_setting() {
        use pt_core::startup::StartupProfile;
        let dir = tempfile::tempdir().unwrap();
        let svc = MonitorService::new(
            Box::new(MockDisplayBackend::new(sample_layout())),
            AppPaths::new(dir.path()),
        );
        svc.save_current("Desk", None, false).unwrap();
        let fast = |svc: &MonitorService| run_sign_in(svc, 3, Duration::ZERO);

        let o = fast(&svc);
        assert!(o.ok && o.message.contains("leaving the monitors"), "{o:?}");

        let mut s = svc.settings().unwrap();
        s.startup_profile = StartupProfile::Profile("Desk".into());
        svc.save_settings(&s).unwrap();
        let o = fast(&svc);
        assert!(o.message.contains("already on screen"), "{o:?}");

        svc.rescue().unwrap(); // screen no longer matches "Desk"
        let o = fast(&svc);
        assert!(o.ok && o.message == "Sign-in: Applied 'Desk'", "{o:?}");
    }

    #[test]
    fn sign_in_waits_for_missing_monitors_then_applies_anyway() {
        use pt_core::startup::StartupProfile;
        let dir = tempfile::tempdir().unwrap();
        let mut only_left = sample_layout();
        only_left.monitors.truncate(1); // RIGHT and TV not plugged in (yet)
        let svc = MonitorService::new(Box::new(MockDisplayBackend::new(only_left)), AppPaths::new(dir.path()));
        let p = pt_core::Profile::new("Both", sample_layout()).unwrap();
        svc.store().save(&p, false, &[]).unwrap();
        let mut s = svc.settings().unwrap();
        s.startup_profile = StartupProfile::Profile("Both".into());
        svc.save_settings(&s).unwrap();
        let o = run_sign_in(&svc, 3, Duration::ZERO);
        assert!(o.message.starts_with("Sign-in (not every monitor showed up)"), "{o:?}");
    }

    #[test]
    fn worker_thread_runs_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let layout = dir.path().join("monitors.toml");
        std::fs::write(&layout, toml::to_string(&sample_layout()).unwrap()).unwrap();
        // The worker builds its backend from the environment, like the real app.
        // SAFETY: set before the worker thread starts; tests in this binary
        // that read this variable all point it at a valid layout file.
        unsafe { std::env::set_var(pt_core::mock::MOCK_LAYOUT_ENV, &layout) };

        let (tx, rx) = mpsc::channel();
        let worker = Worker::spawn(AppPaths::new(dir.path().join("cfg")), move |o| tx.send(o).unwrap());
        worker.send(Job::Extend);
        let o = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(o.ok, "{o:?}");
    }
}
