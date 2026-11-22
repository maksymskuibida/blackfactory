//! A-19 (U-76): the texts of the retry verdict stay keys of the strings file. The request, the review
//! and the second look they belong to are removed (D-115), so only the keys are held here.

use std::fs;

#[test]
fn every_new_text_is_a_key_of_the_strings_file() {
    let text = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/strings/en/scope_retry.toml"
    ))
    .unwrap();
    for key in [
        "scope.retry.verdict.must",
        "scope.retry.request.spent",
        "scope.retry.request.same_patch",
        "scope.retry.brief.file",
        "scope.retry.review.question",
        "scope.retry.row.title",
        "scope.retry.row.spent_title",
    ] {
        assert!(text.contains(&format!("\"{key}\"")), "{key}");
    }
}
