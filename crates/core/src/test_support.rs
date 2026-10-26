//! Test scaffolding shared by unit and integration tests.
//!
//! Not part of the public API and exempt from semver. It is compiled
//! unconditionally rather than behind `#[cfg(test)]` because integration tests
//! in `tests/` link against this crate as an external consumer and therefore
//! cannot see `#[cfg(test)]` items. That gap is what let an integration test
//! overwrite the developer's real `~/.minutes/search.db` (#588): the one test
//! that reached global state was the one with no way to isolate it.
//!
//! Several paths are resolved from the home directory rather than from
//! [`crate::config::Config`], notably `search.db` and `graph.db`. A test that
//! isolates only `output_dir` therefore still writes to real user state. Wrap
//! any such test in [`with_temp_home`].

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Whether this process is a test binary built by `cargo test`.
///
/// `cfg(test)` only reaches this crate's own unit tests. Integration tests in
/// `tests/` and the test binaries of other crates (the desktop app, the CLI)
/// link this crate as an ordinary dependency, so they are recognized by where
/// Cargo puts them instead: `<target>/<profile>/deps/<crate>-<16 hex>`.
/// Installed binaries, `cargo run` builds (`<target>/<profile>/<name>`) and
/// the bundled app never match that shape, so runtime behavior is unchanged.
pub fn running_under_cargo_test() -> bool {
    if cfg!(test) {
        return true;
    }
    static DETECTED: OnceLock<bool> = OnceLock::new();
    *DETECTED.get_or_init(|| {
        std::env::current_exe()
            .map(|exe| is_cargo_test_binary(&exe))
            .unwrap_or(false)
    })
}

fn is_cargo_test_binary(exe: &Path) -> bool {
    let in_deps = exe.parent().and_then(Path::file_name) == Some(OsStr::new("deps"));
    let hashed_stem = exe
        .file_stem()
        .and_then(OsStr::to_str)
        .and_then(|stem| stem.rsplit_once('-'))
        .is_some_and(|(name, hash)| {
            !name.is_empty() && hash.len() == 16 && hash.bytes().all(|b| b.is_ascii_hexdigit())
        });
    in_deps && hashed_stem
}

/// Serialize tests that mutate the `HOME` environment variable.
///
/// The environment is process-global, so concurrent tests would otherwise
/// observe each other's overrides. Poisoning is ignored: a panicking test
/// leaves the lock poisoned but the guard itself is still sound to hand out.
pub fn home_env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Sets `HOME` for as long as it is held, restoring the previous value on drop.
///
/// Take [`home_env_lock`] first. Prefer [`with_temp_home`], which does both.
pub struct HomeOverride {
    previous: Option<OsString>,
}

impl HomeOverride {
    pub fn set(path: &Path) -> Self {
        let previous = std::env::var_os("HOME");
        std::env::set_var("HOME", path);
        Self { previous }
    }
}

impl Drop for HomeOverride {
    fn drop(&mut self) {
        if let Some(previous) = &self.previous {
            std::env::set_var("HOME", previous);
        } else {
            std::env::remove_var("HOME");
        }
    }
}

/// A temporary `HOME`, isolated for as long as this value is held.
///
/// [`with_temp_home`] covers the common case and should be preferred. Use this
/// when the isolation has to outlive a closure, such as a fixture that builds a
/// [`crate::config::Config`] which the test body then uses across many
/// statements.
///
/// Holding this serializes against every other `HOME` mutator, which is the
/// price of `HOME` being process-global. Dropping it restores the previous
/// `HOME` before releasing the lock, so the next waiter never observes this
/// test's home.
pub struct TempHome {
    // Field order is drop order: restore HOME, then release the lock, then
    // remove the directory.
    _home: HomeOverride,
    _lock: MutexGuard<'static, ()>,
    temp: tempfile::TempDir,
}

impl TempHome {
    /// Points `HOME` at a fresh temporary directory until the value is dropped.
    pub fn new() -> Self {
        let lock = home_env_lock();
        let temp = tempfile::tempdir().expect("temp home");
        let home = HomeOverride::set(temp.path());
        Self {
            _home: home,
            _lock: lock,
            temp,
        }
    }

    /// The temporary home directory.
    pub fn path(&self) -> &Path {
        self.temp.path()
    }
}

impl Default for TempHome {
    fn default() -> Self {
        Self::new()
    }
}

/// Run `f` with `HOME` pointed at a fresh temporary directory.
///
/// Anything the body resolves from the home directory (`search.db`, `graph.db`,
/// `~/.minutes/...`) lands in that directory and is discarded afterwards, so the
/// developer's real state is never touched. The lock is held for the duration,
/// so tests using this run one at a time.
pub fn with_temp_home<T>(f: impl FnOnce(&Path) -> T) -> T {
    let _guard = home_env_lock();
    let temp = tempfile::tempdir().expect("temp home");
    let _home = HomeOverride::set(temp.path());
    f(temp.path())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_test_binaries_are_recognized_by_their_target_layout() {
        for exe in [
            "/repo/target/debug/deps/minutes_core-0123456789abcdef",
            "/repo/target/release/deps/minutes_app-fedcba9876543210",
            "/repo/target/debug/deps/integration-00000000deadbeef",
            "C:/repo/target/debug/deps/minutes_app-0123456789abcdef.exe",
        ] {
            assert!(is_cargo_test_binary(Path::new(exe)), "{exe}");
        }
    }

    #[test]
    fn shipped_and_cargo_run_binaries_are_not_test_binaries() {
        for exe in [
            "/repo/target/debug/minutes",
            "/repo/target/debug/minutes-app",
            "/Applications/Minutes.app/Contents/MacOS/minutes-app",
            "/opt/homebrew/bin/minutes",
            "/repo/target/debug/deps/minutes",
            "/repo/target/debug/deps/minutes-app",
            "/repo/target/debug/deps/minutes-0123456789abcdeg",
            "/repo/target/debug/deps/-0123456789abcdef",
            "/home/me/bin/minutes-0123456789abcdef",
        ] {
            assert!(!is_cargo_test_binary(Path::new(exe)), "{exe}");
        }
    }

    #[test]
    fn this_crates_unit_tests_count_as_a_test_harness() {
        assert!(running_under_cargo_test());
    }
}
