use pdt_core::activity::{CODEX_APPROVAL_MARKERS, screen_shows, squash, strip_ansi};

#[test]
fn approval_screen_is_recognised_in_a_real_capture() {
    let raw = include_bytes!("fixtures/codex-approval.txt");
    let screen = squash(&strip_ansi(raw));
    assert!(screen_shows(&screen, CODEX_APPROVAL_MARKERS), "markers missing from the Task 1 capture");
    // Every verified marker survives on its own (the edit-approval marker is unverified, so it is not checked).
    for m in &CODEX_APPROVAL_MARKERS[..4] {
        assert!(screen_shows(&screen, &[m]), "{m:?} missing from the Task 1 capture");
    }
    let before_the_prompt = &raw[..raw.len() / 4];
    assert!(!screen_shows(&squash(&strip_ansi(before_the_prompt)), CODEX_APPROVAL_MARKERS), "no false positive");
}
