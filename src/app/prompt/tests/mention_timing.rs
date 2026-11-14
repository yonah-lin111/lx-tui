//! 提及面板的打开时机：只有文本编辑能新开会话，纯移动与根同步只收敛已存在的会话。

use super::*;

/// 生产一条已确认的 `@src/app.rs `，再敲一个空格解除压制（模拟“继续输入正文”）。
fn confirmed_mention_with_cache() -> Prompt {
    let mut prompt = mention_prompt(vec![file("src/app.rs"), file("src/main.rs")]);
    assert!(prompt.mention_confirm());
    assert_eq!(prompt.text(), "@src/app.rs ");
    assert!(prompt.mention().is_none(), "确认后面板关闭");
    prompt.insert_char(' ');
    assert!(prompt.mention().is_none(), "尾随空格后不在触发词内");
    prompt
}

#[test]
fn cursor_move_into_existing_mention_does_not_open_panel() {
    let mut prompt = confirmed_mention_with_cache();
    let revision = prompt.revision();
    // 从文末逐字左移进入 `@src/app.rs` 内部。
    for _ in 0..6 {
        prompt.move_left();
    }
    assert!(prompt.mention().is_none(), "纯光标移动不得重新打开提及面板");
    assert_eq!(prompt.revision(), revision, "移动不改变修订号");
}

#[test]
fn mouse_move_into_existing_mention_does_not_open_panel() {
    let mut prompt = confirmed_mention_with_cache();
    prompt.set_cursor_offset(6);
    assert!(prompt.mention().is_none(), "点击/定位不新开面板");
    prompt.set_cursor_from_cell(0, 5);
    assert!(prompt.mention().is_none(), "单元格落点不新开面板");
}

#[test]
fn sync_root_does_not_open_closed_mention_panel() {
    let mut prompt = confirmed_mention_with_cache();
    prompt.set_cursor_offset(6);
    prompt.set_mention_root(Some(PathBuf::from("/tmp/ws")));
    assert!(
        prompt.mention().is_none(),
        "按键前的根同步不得绕过移动规则新开面板"
    );
}

#[test]
fn typing_inside_existing_mention_opens_panel() {
    let mut prompt = confirmed_mention_with_cache();
    prompt.set_cursor_offset(6);
    assert!(prompt.mention().is_none());
    prompt.insert_char('p');
    assert_eq!(prompt.text(), "@src/appp.rs  ");
    assert!(
        prompt.mention().is_some(),
        "在已有 @ 路径中间打字按编辑语义重开面板"
    );
    assert_eq!(
        prompt.mention_trigger().map(|t| t.query.as_str()),
        Some("src/ap")
    );
}

#[test]
fn edit_after_mention_confirm_reopens_on_first_keystroke() {
    let mut prompt = mention_prompt(vec![file("src/app.rs")]);
    assert!(prompt.mention_confirm());
    assert_eq!(prompt.text(), "@src/app.rs ");
    assert!(prompt.mention().is_none());

    // 复现事件层顺序：先同步根（把 panel_suppressed 镜像进 mention），再插入字符。
    prompt.set_mention_root(Some(PathBuf::from("/tmp/ws")));
    prompt.insert_char('@');

    assert!(
        prompt.mention().is_some(),
        "确认后的第一次编辑必须当次重开面板，不能延迟一拍"
    );
    assert_eq!(prompt.mention_trigger().map(|t| t.from), Some(12));
}

#[test]
fn escape_then_move_stays_closed_until_edit() {
    let mut prompt = mention_prompt(vec![file("main.rs")]);
    prompt.insert_str("main");
    assert!(prompt.mention_escape());
    assert!(prompt.mention().is_none());
    for _ in 0..4 {
        prompt.move_left();
    }
    assert!(prompt.mention().is_none(), "Esc 后移动不解除压制");
    prompt.insert_char('i');
    assert!(
        prompt.mention_trigger().is_some(),
        "文本再次变化才重新武装会话"
    );
    prompt.backspace();
    assert!(prompt.mention().is_some(), "过滤词回到 main 后列出候选");
}

#[test]
fn root_change_while_suppressed_stays_closed() {
    let mut prompt = mention_prompt(vec![file("src/app.rs")]);
    assert!(prompt.mention_escape());
    assert!(prompt.mention().is_none());
    prompt.set_mention_root(Some(PathBuf::from("/tmp/other")));
    assert!(
        prompt.mention_trigger().is_none(),
        "根同步不得解除 Esc/确认后的压制"
    );
    assert!(prompt.mention().is_none());
}

#[test]
fn scan_result_after_move_stays_closed() {
    let mut prompt = prompt(40, 8);
    prompt.set_mention_root(Some(PathBuf::from("/tmp/ws")));
    prompt.insert_char('@');
    let (generation, _) = prompt.take_mention_scan_request().expect("scan requested");
    // 移动到触发点之前关闭会话，再移回；纯移动不得重建会话。
    prompt.move_left();
    assert!(prompt.mention_trigger().is_none());
    prompt.move_right();
    assert!(prompt.apply_mention_entries(generation, vec![file("app.rs")]));
    assert!(
        prompt.mention().is_none(),
        "迟到的扫描结果不得重开已关闭的会话"
    );
    // 下一次文本编辑才按缓存重开。
    prompt.insert_char('a');
    assert!(prompt.mention().is_some());
}
