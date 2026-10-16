use pdt_core::activity::{
    CODEX_APPROVAL_MARKERS, codex_approval_shown, codex_markers_shown, osc_titles, screen_shows, squash, strip_ansi,
    titles_say_action_required,
};

#[test]
fn approval_screen_is_recognised_in_a_real_capture() {
    let raw = include_bytes!("fixtures/codex-approval.txt");
    let screen = squash(&strip_ansi(raw));
    assert!(screen_shows(&screen, CODEX_APPROVAL_MARKERS), "markers missing from the Task 1 capture");
    // Every verified marker survives on its own (the edit-approval marker is unverified, so it is not checked).
    for m in &CODEX_APPROVAL_MARKERS[..4] {
        assert!(screen_shows(&screen, &[m]), "{m:?} missing from the Task 1 capture");
    }
    assert!(codex_markers_shown(&screen) >= 2);
    assert!(titles_say_action_required(&osc_titles(raw).0), "the capture's window title says Action Required");
    assert!(codex_approval_shown(false, &screen), "the screen alone is enough");
    let before_the_prompt = &raw[..raw.len() / 4];
    assert!(
        !codex_approval_shown(
            titles_say_action_required(&osc_titles(before_the_prompt).0),
            &squash(&strip_ansi(before_the_prompt))
        ),
        "no false positive"
    );
}
