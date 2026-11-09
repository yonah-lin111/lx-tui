use super::super::{set_test_env, test_home_override};
use super::resolve_log_dir;
use std::path::{Path, PathBuf};

fn temp_home(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("lx-log-dir-{label}-{}-{nanos}", std::process::id()))
}

struct HomeGuard(PathBuf);

impl Drop for HomeGuard {
    fn drop(&mut self) {
        set_test_env(None, None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn dev_and_prod_roots_are_separate() {
    let home = Path::new("/home/tester");
    assert_eq!(
        resolve_log_dir(home, true, None),
        PathBuf::from("/home/tester/.lx-t-dev/logs")
    );
    assert_eq!(
        resolve_log_dir(home, false, None),
        PathBuf::from("/home/tester/.lx-t/logs")
    );
}

#[test]
fn explicit_override_wins() {
    let home = Path::new("/home/tester");
    assert_eq!(
        resolve_log_dir(home, true, Some(Path::new("/tmp/custom-logs"))),
        PathBuf::from("/tmp/custom-logs")
    );
}

#[test]
fn test_home_override_is_used() {
    let home = temp_home("override");
    std::fs::create_dir_all(&home).expect("create temp home");
    set_test_env(Some(home.clone()), None);
    let _guard = HomeGuard(home.clone());
    assert_eq!(test_home_override(), Some(home.clone()));
    let dir = super::log_dir().expect("log dir");
    assert_eq!(dir, home.join(".lx-t-dev").join("logs"));
}
