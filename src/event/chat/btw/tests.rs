use super::target_view_for;

#[test]
fn default_target_key_maps_to_the_default_assistant_not_an_agent_id() {
    let view = target_view_for("default");
    assert!(view.agent_id.is_none());
}
