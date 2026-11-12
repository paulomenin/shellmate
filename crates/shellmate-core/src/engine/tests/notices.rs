//! What a request tells the person, as values: the tone and symbol of each notice while it runs, and the
//! kind of each note after the answer. How they look is a presenter's business; these say what they mean.

use super::*;
use crate::present::{Mark, Notice, Tone};

async fn notice_of(call: &str, approver: Option<Arc<FakeApprover>>) -> Notice {
    // the default config turns `man` on; the bare test engine has it off
    let mut e = engine(vec![("a", FakeProvider::replying("x"), 1)])
        .with_config(Config::defaults())
        .with_builtin_tools();
    if let Some(a) = approver {
        e = e.with_approver(a);
    }
    e.run_tool(call, None, false, "`a`").await.notice
}

#[tokio::test]
async fn a_command_that_ran_is_a_success_with_its_output_as_detail() {
    let n = notice_of("exec echo hi", Some(FakeApprover::new(Choice::Run))).await;
    assert_eq!((n.tone, n.mark), (Tone::Success, Mark::Check));
    assert_eq!(n.text, "ran (approved): echo hi");
    assert!(
        n.meta.as_deref().unwrap().starts_with("exit 0,"),
        "{:?}",
        n.meta
    );
    assert_eq!(n.detail, ["hi"]);
}

#[tokio::test]
async fn a_command_that_failed_is_a_warning_even_though_it_ran() {
    let n = notice_of("exec false", Some(FakeApprover::new(Choice::Run))).await;
    assert_eq!((n.tone, n.mark), (Tone::Warning, Mark::Check));
    assert!(
        n.meta.as_deref().unwrap().starts_with("exit 1,"),
        "{:?}",
        n.meta
    );
}

#[tokio::test]
async fn a_command_the_person_denied_is_a_warning_with_a_cross() {
    let n = notice_of("exec echo hi", Some(FakeApprover::new(Choice::Deny))).await;
    assert_eq!((n.tone, n.mark), (Tone::Warning, Mark::Cross));
    assert_eq!(n.text, "denied: echo hi");
}

#[tokio::test]
async fn a_command_moved_to_the_prompt_is_information() {
    let n = notice_of("exec echo hi", Some(FakeApprover::new(Choice::ToBuffer))).await;
    assert_eq!((n.tone, n.mark), (Tone::Info, Mark::Arrow));
    assert_eq!(n.text, "moved to your prompt: echo hi");
}

#[tokio::test]
async fn a_refusal_is_a_warning_with_a_chevron() {
    // nobody to ask, a complex line, an unknown tool and an unreadable call are all refusals
    for call in [
        "exec echo hi",
        "exec ls | wc",
        "curl http://example.com",
        "",
    ] {
        let n = notice_of(call, None).await;
        assert_eq!((n.tone, n.mark), (Tone::Warning, Mark::Chevron), "{call}");
    }
    let n = notice_of("exec echo hi", None).await;
    assert_eq!(n.text, "denied: exec echo hi");
    assert_eq!(notice_of("", None).await.text, "(invalid tool call)");
}

#[tokio::test]
async fn a_manual_page_is_background_detail() {
    let n = notice_of("man sm-no-such-page", None).await;
    assert_eq!((n.tone, n.mark), (Tone::Muted, Mark::Chevron));
    assert_eq!(n.text, "man sm-no-such-page");
}

async fn notes_of(reply: &str) -> Vec<crate::present::Remark> {
    let e = engine(vec![("a", FakeProvider::replying(reply), 1)]);
    e.run(&env("ask", "do it")).await.unwrap().notes
}

#[tokio::test]
async fn a_second_command_is_listed_as_other_options() {
    let notes =
        notes_of("Two ways.\n```shellmate-command\nls\n```\n```shellmate-command\nls -a\n```")
            .await;
    assert_eq!(notes.len(), 1);
    assert_eq!((notes[0].tone, notes[0].label), (Tone::Info, None));
    assert_eq!(notes[0].text, "Other options:");
    assert_eq!(notes[0].items, ["ls -a"]);
    assert_eq!(notes[0].plain(), "Other options:\n  ls -a");
}

#[tokio::test]
async fn a_dropped_suggestion_is_a_warning() {
    let notes = notes_of("Careful.\n```shellmate-command\necho \u{1b}[2J\n```").await;
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].tone, Tone::Warning);
    assert!(notes[0].text.starts_with("(dropped 1 suggestion(s)"));
}

#[tokio::test]
async fn a_finding_about_the_command_is_a_labelled_warning() {
    let notes = notes_of("Try.\n```shellmate-command\ndefinitely-not-a-program --x\n```").await;
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(
        (notes[0].tone, notes[0].label),
        (Tone::Warning, Some("check"))
    );
    assert!(notes[0].text.contains("was not found in PATH"));
}

#[tokio::test]
async fn a_clean_answer_has_no_notes() {
    assert!(notes_of("Done.\n```shellmate-command\nls\n```")
        .await
        .is_empty());
}
