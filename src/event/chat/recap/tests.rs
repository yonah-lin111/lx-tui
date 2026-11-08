use super::*;

#[test]
fn no_recap_effect_is_noop() {
    let mut state = AppState::demo();
    let (sender, _receiver) = mpsc::unbounded_channel();
    assert!(!apply_recap_effects(&mut state, &sender));
}
