use super::*;
use std::sync::Mutex;

/// 内存 recorder：记录 prepare/commit 顺序，模拟持久化 ACK。
#[derive(Default)]
struct MemoryRecorder {
    prepared: Mutex<Vec<PreparedMutation>>,
    committed: Mutex<Vec<ObservedMutation>>,
    fail_prepare: bool,
}

impl MutationRecorder for MemoryRecorder {
    fn prepare(&self, mutation: &PreparedMutation) -> Result<MutationTicket, MutationError> {
        if self.fail_prepare {
            return Err(MutationError::new("denied", "snapshot rejected"));
        }
        let mut prepared = self.prepared.lock().expect("lock");
        let ids = (prepared.len() as i64 + 1..)
            .take(mutation.paths.len())
            .collect();
        prepared.push(mutation.clone());
        Ok(MutationTicket { ids })
    }

    fn commit(
        &self,
        _ticket: MutationTicket,
        observed: &ObservedMutation,
    ) -> Result<(), MutationError> {
        self.committed
            .lock()
            .expect("commit")
            .push(observed.clone());
        Ok(())
    }
}

fn temp_file(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "lx-mutation-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::write(&path, b"before").expect("seed");
    path
}

#[test]
fn prepare_failure_leaves_target_untouched() {
    let path = temp_file("reject");
    let recorder = MemoryRecorder {
        fail_prepare: true,
        ..MemoryRecorder::default()
    };
    let before = read_entry(&path).expect("read");
    let change = PathMutation {
        operation: MutationOperation::Update,
        path: path.clone(),
        target: path.clone(),
        before,
        planned_after: RawImage::regular(b"after".to_vec(), None),
    };
    let result = capture(Some(&recorder), vec![change], || {
        std::fs::write(&path, b"after")
    });
    assert!(result.is_err());
    assert_eq!(std::fs::read(&path).expect("read"), b"before");
    let _ = std::fs::remove_file(path);
}

#[test]
fn commit_observes_actual_bytes_even_on_writer_error() {
    let path = temp_file("observed");
    let recorder = MemoryRecorder::default();
    let before = read_entry(&path).expect("read");
    let change = PathMutation {
        operation: MutationOperation::Update,
        path: path.clone(),
        target: path.clone(),
        before,
        planned_after: RawImage::regular(b"after".to_vec(), None),
    };
    let result = capture(Some(&recorder), vec![change], || {
        std::fs::write(&path, b"partially")?;
        Err(std::io::Error::other("boom"))
    });
    assert!(result.is_err());
    let committed = recorder.committed.lock().expect("commit");
    assert_eq!(committed.len(), 1);
    assert_eq!(committed[0].paths[0].actual_after.bytes, b"partially");
    assert!(!committed[0].paths[0].success);
    let _ = std::fs::remove_file(path);
}

#[cfg(unix)]
#[test]
fn hard_linked_file_is_refused() {
    let path = temp_file("hardlink");
    let other = path.with_extension("link");
    std::fs::hard_link(&path, &other).expect("hard link");
    assert!(
        ensure_single_link(&path).is_err(),
        "多硬链接对象不得静默按普通文件捕获"
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&other);
}
