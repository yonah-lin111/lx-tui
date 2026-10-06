//! 单元测试；仅测试构建编译。

use super::*;
use std::cell::RefCell;
use std::rc::Rc;

/// 共享输出缓冲：闭包按写入长度记录步骤发生的位置。
#[derive(Clone, Default)]
struct SharedWriter(Rc<RefCell<Vec<u8>>>);

impl Write for SharedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// 复现退出后鼠标上报泄漏进 shell：drain 必须在 cooked 模式之前、备用屏退出之前完成。
#[test]
fn restore_disables_mouse_before_draining_and_restores_cooked_last() {
    let sink = SharedWriter::default();
    let steps: Rc<RefCell<Vec<(&'static str, usize)>>> = Rc::new(RefCell::new(Vec::new()));
    let mut drain = {
        let sink = sink.clone();
        let steps = Rc::clone(&steps);
        move || {
            steps.borrow_mut().push(("drain", sink.0.borrow().len()));
        }
    };
    let mut restore_cooked = {
        let sink = sink.clone();
        let steps = Rc::clone(&steps);
        move || {
            steps.borrow_mut().push(("cooked", sink.0.borrow().len()));
            Ok(())
        }
    };

    let mut writer = sink.clone();
    restore_terminal(&mut writer, &mut drain, &mut restore_cooked)
        .expect("restore sequence succeeds");

    let output = String::from_utf8(sink.0.borrow().clone()).expect("output is utf8");
    let mouse_off = output
        .find("\x1b[?1003l")
        .expect("mouse reporting disabled");
    let keyboard_off = output
        .find("\x1b[<1u")
        .expect("keyboard enhancement popped");
    let leave_screen = output.find("\x1b[?1049l").expect("alternate screen left");
    let cursor_reset = output.find("\x1b[?12h").expect("cursor blink restored");
    let steps = steps.borrow();
    let step_at = |name: &str| {
        steps
            .iter()
            .find(|(step, _)| *step == name)
            .map(|(_, at)| *at)
            .unwrap_or_else(|| panic!("missing step {name}"))
    };
    let drain_at = step_at("drain");
    let cooked_at = step_at("cooked");

    assert!(
        mouse_off < drain_at,
        "drain must run after mouse reporting is disabled"
    );
    assert!(
        drain_at <= keyboard_off,
        "keyboard enhancement must be popped after draining"
    );
    assert!(
        keyboard_off < leave_screen,
        "keyboard enhancement must be popped before leaving the alternate screen"
    );
    assert!(
        drain_at <= leave_screen,
        "drain must run before leaving the alternate screen"
    );
    assert!(
        leave_screen < cooked_at,
        "cooked mode must be restored after leaving the alternate screen"
    );
    assert!(
        cursor_reset < cooked_at,
        "cursor blink must be restored before leaving raw mode"
    );
}
