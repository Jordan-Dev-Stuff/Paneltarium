//! Running the display tool of a desktop (`kscreen-doctor`, `wlr-randr`,
//! `xrandr`). Behind a trait so tests can play back recorded output and
//! check the arguments that would be sent.

use std::path::Path;
use std::process::Command;

use pt_core::{Error, Result};

pub trait Runner: Send + Sync {
    /// Run `program` with `args`; its standard output on success.
    fn run(&self, program: &'static str, args: &[String]) -> Result<String>;
}

/// Runs the real programs.
pub struct System;

impl Runner for System {
    fn run(&self, program: &'static str, args: &[String]) -> Result<String> {
        let out = Command::new(program).args(args).output().map_err(|e| Error::Os {
            call: program,
            code: e.raw_os_error().map(i64::from),
            message: format!("couldn't run {program}: {e}"),
        })?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let detail = [stderr.trim(), stdout.trim()]
                .into_iter()
                .find(|s| !s.is_empty())
                .unwrap_or("no output");
            return Err(Error::Os {
                call: program,
                code: out.status.code().map(i64::from),
                message: detail.lines().take(5).collect::<Vec<_>>().join(" / "),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// Is `program` somewhere on `PATH`?
pub fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| is_executable(&dir.join(program))))
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

/// `1.0` → "1", `1.25` → "1.25": how the display tools like numbers.
pub fn number(v: f64) -> String {
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Plays back canned output for a read and records every call.
    pub struct Fake {
        pub output: String,
        pub calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl Fake {
        pub fn new(output: &str) -> Self {
            Fake {
                output: output.into(),
                calls: Mutex::new(Vec::new()),
            }
        }

        pub fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl Runner for std::sync::Arc<Fake> {
        fn run(&self, program: &'static str, args: &[String]) -> Result<String> {
            self.calls.lock().unwrap().push((program.into(), args.to_vec()));
            Ok(self.output.clone())
        }
    }

    #[test]
    fn numbers_are_short() {
        assert_eq!(number(1.0), "1");
        assert_eq!(number(1.25), "1.25");
        assert_eq!(number(1.5000001), "1.5");
        assert_eq!(number(2.0), "2");
    }

    #[test]
    fn finds_programs_on_path() {
        assert!(on_path("sh"));
        assert!(!on_path("paneltarium-no-such-program"));
    }

    #[test]
    fn failing_programs_report_their_error() {
        let err = System
            .run("sh", &["-c".into(), "echo 'bad mode' >&2; exit 3".into()])
            .unwrap_err();
        let text = err.to_string();
        assert!(text.contains("bad mode") && text.contains("code 3"), "{text}");
        assert_eq!(System.run("sh", &["-c".into(), "echo hi".into()]).unwrap(), "hi\n");
        assert!(System.run("paneltarium-no-such-program", &[]).is_err());
    }
}
