use super::*;

fn ctx() -> Context {
    Context {
        shell: "fish".into(),
        shell_version: Some("4.9.3".into()),
        os: "Arch Linux".into(),
        cwd: Some("~/projects/x".into()),
        buffer: "find big files".into(),
        last_status: Some(2),
        history: vec![(0, "ls".into()), (2, "make build".into())],
        capture: None,
        project: None,
        git: None,
        git_untrusted: false,
    }
}

#[test]
fn project_and_git_sections_follow_their_levels() {
    let mut c = ctx();
    c.project = Some(crate::collect::ProjectInfo {
        kinds: vec!["PROJECT-SECTION"],
        files: vec!["f"],
    });
    c.git = Some("GIT-SECTION\n".into());
    let sys = |level| build(Action::Ask, &c, level).unwrap()[0].content.clone();
    assert!(!sys(0).contains("PROJECT-SECTION") && !sys(0).contains("GIT-SECTION"));
    assert!(sys(1).contains("PROJECT-SECTION") && !sys(1).contains("GIT-SECTION"));
    assert!(sys(2).contains("PROJECT-SECTION") && sys(2).contains("GIT-SECTION"));
}

#[test]
fn captured_output_is_sent_only_at_level_two() {
    let mut c = ctx();
    c.capture = Some("CAPTURED-SECTION\n".into());
    assert!(!build(Action::Ask, &c, 1).unwrap()[0]
        .content
        .contains("CAPTURED-SECTION"));
    assert!(build(Action::Ask, &c, 2).unwrap()[0]
        .content
        .contains("CAPTURED-SECTION"));
}

#[test]
fn the_command_block_rules_say_how_steps_and_alternatives_are_shown() {
    let m = build(Action::Ask, &ctx(), 1).unwrap();
    let sys = &m[0].content;
    // steps go in one block, `&&` when one needs the other, a second block only for an alternative
    assert!(sys.contains("Steps that belong together go in that one block"));
    assert!(sys.contains("`&&`"));
    // the model is told the exact heading the user will see over an extra block
    assert!(sys.contains(&format!("\"{}\"", crate::response::OTHER_OPTIONS_LABEL)));
}

#[test]
fn level_one_includes_environment_and_dialect() {
    let m = build(Action::Generate, &ctx(), 1).unwrap();
    assert!(m[0].content.contains("fish 4.9.3 on Arch Linux"));
    assert!(m[0].content.contains("~/projects/x"));
    assert!(m[0].content.contains("`set`"));
    assert!(m[0].content.contains("shellmate-command"));
    assert_eq!(m[1].content, "find big files");
}

#[test]
fn history_only_at_level_two() {
    let one = build(Action::Ask, &ctx(), 1).unwrap();
    assert!(!one[0].content.contains("make build") && !one[0].content.contains("status 2"));
    let two = build(Action::Ask, &ctx(), 2).unwrap();
    assert!(two[0].content.contains("exited with status 2"));
    assert!(two[0].content.contains("  2 make build"));
}

#[test]
fn history_window_keeps_the_most_recent() {
    let env = Envelope {
        hist: (0..9).map(|i| (0, format!("cmd{i}"))).collect(),
        ..Default::default()
    };
    let c = Context::from_envelope(&env, "os".into(), None, 3);
    assert_eq!(
        c.history.iter().map(|h| h.1.as_str()).collect::<Vec<_>>(),
        ["cmd6", "cmd7", "cmd8"]
    );
}

#[test]
fn level_zero_sends_no_environment() {
    let m = build(Action::Ask, &ctx(), 0).unwrap();
    assert!(!m[0].content.contains("fish") || !m[0].content.contains("Arch"));
    assert!(!m[0].content.contains("~/projects/x"));
}

#[test]
fn history_sits_between_the_system_prompt_and_the_request() {
    let history = [Message::user("earlier q"), Message::assistant("earlier a")];
    let m = build_with(
        Action::Ask,
        &ctx(),
        1,
        &Extras {
            history: &history,
            omitted: 2,
            ..Extras::default()
        },
    )
    .unwrap();
    let roles: Vec<_> = m.iter().map(|x| x.content.as_str()).collect();
    assert_eq!(roles[1], "earlier q");
    assert_eq!(roles[2], "earlier a");
    assert_eq!(roles[3], "find big files");
    assert!(m[0].content.contains("2 earlier turn(s)"));
}

#[test]
fn tool_instructions_are_in_the_prompt_only_when_given() {
    let on = build_with(
        Action::Ask,
        &ctx(),
        1,
        &Extras {
            tool_offers: &["You can use a tool.\n".to_string()],
            ..Extras::default()
        },
    )
    .unwrap();
    assert!(on[0].content.contains("You can use a tool.\n"));
    let off = build(Action::Ask, &ctx(), 1).unwrap();
    assert!(!off[0].content.contains("You can use a tool."));
}

#[test]
fn an_unchanged_suggestion_is_flagged_for_follow_ups() {
    let m = build_with(
        Action::Ask,
        &ctx(),
        1,
        &Extras {
            unchanged_suggestion: true,
            ..Extras::default()
        },
    )
    .unwrap();
    assert!(m
        .last()
        .unwrap()
        .content
        .contains("suggested last, unchanged"));
}

#[test]
fn empty_buffer_is_an_error() {
    let mut c = ctx();
    c.buffer = "  \n".into();
    assert_eq!(build(Action::Ask, &c, 1).unwrap_err().code, "input.empty");
}

#[test]
fn home_is_abbreviated() {
    assert_eq!(tilde("/home/p/x", Some("/home/p")), "~/x");
    assert_eq!(tilde("/home/p", Some("/home/p")), "~");
    assert_eq!(tilde("/home/pp/x", Some("/home/p")), "/home/pp/x");
    assert_eq!(tilde("/etc", None), "/etc");
}
