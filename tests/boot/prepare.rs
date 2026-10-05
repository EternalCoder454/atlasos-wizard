//! `prepare`: one test per row of the decision table, and the power-cut cases.

use super::*;
use std::time::Instant;

const VERIFIED: &str = r#"{"format":1,"boots":BOOTS,"gave_up":false,"account":{"name":"ada","uid":1000,"stage":"verified"}}"#;

fn verified(boots: u32) -> String {
    VERIFIED.replace("BOOTS", &boots.to_string())
}

// ---- row 1: a done marker

#[test]
fn done_machine_gets_cleaned_up_and_locked() {
    let s = Sys::new();
    s.markers();
    s.add_human("ada", 1000);
    s.write("etc/plasmalogin.conf.d/99-atlas-wizard.conf", DROPIN);
    s.write("run/atlas-setup/answers.json", "{}");
    s.prepare();
    assert_eq!(s.dropin(), None, "the setup autologin is gone");
    assert_eq!(s.commands(), LOCK_CMDS);
    assert!(s.files_under("run/atlas-setup").is_empty());
    assert!(
        s.read("etc/shadow")
            .contains("atlas-setup:!*:19000:0:99999:7::0:")
    );
    assert!(s.read("etc/passwd").contains("/usr/sbin/nologin"));
    assert!(
        !s.exists("var/lib/atlas-wizard"),
        "a done boot writes no state"
    );
}

#[test]
fn second_done_boot_runs_nothing_and_writes_nothing() {
    let s = Sys::new();
    s.markers();
    s.prepare();
    let after_first = s.commands();
    let shadow = s.read("etc/shadow");
    let marker = fs::metadata(s.path("etc/atlasos/setup-done"))
        .unwrap()
        .modified()
        .unwrap();
    s.prepare();
    s.prepare();
    assert_eq!(s.commands(), after_first, "no command on a locked machine");
    assert_eq!(s.read("etc/shadow"), shadow);
    assert_eq!(
        fs::metadata(s.path("etc/atlasos/setup-done"))
            .unwrap()
            .modified()
            .unwrap(),
        marker
    );
    assert!(!s.exists("var"));
}

#[test]
fn marker_present_but_dropin_still_there_is_a_power_cut_after_finish() {
    let s = Sys::new();
    s.markers();
    s.lock_atlas_setup();
    s.write("etc/plasmalogin.conf.d/99-atlas-wizard.conf", DROPIN);
    s.prepare();
    assert_eq!(s.dropin(), None);
    assert!(s.commands().is_empty(), "already locked: nothing to run");
}

#[test]
fn only_our_marker_writes_plasmas() {
    let s = Sys::new();
    s.write("etc/atlasos/setup-done", "[Setup]\nVersion=1\n");
    s.prepare();
    assert_eq!(s.marker_state(), (true, true));
    assert!(
        s.read("etc/plasma-setup-done")
            .contains("Setup completed by atlas-wizard")
    );
}

#[test]
fn only_plasmas_marker_means_done_and_writes_ours() {
    let s = Sys::new();
    s.write("etc/plasma-setup-done", "old\n");
    s.prepare();
    assert_eq!(s.marker_state(), (true, true));
    assert_eq!(s.read("etc/plasma-setup-done"), "old\n", "left alone");
    assert!(s.read("etc/atlasos/setup-done").contains("Wizard=0.1.0"));
    assert_eq!(s.dropin(), None);
}

// ---- row 2: atlas.wizard=skip

#[test]
fn skip_with_an_account_marks_done() {
    let s = Sys::new();
    s.cmdline("atlas.wizard=skip");
    s.add_human("ada", 1000);
    s.prepare();
    assert_eq!(s.marker_state(), (true, true));
    assert_eq!(s.dropin(), None);
    assert_eq!(s.commands(), LOCK_CMDS);
}

#[test]
fn skip_without_an_account_still_runs_the_wizard() {
    let s = Sys::new();
    s.cmdline("atlas.wizard=skip");
    s.prepare();
    assert_eq!(s.marker_state(), (false, false));
    assert_eq!(s.dropin().as_deref(), Some(DROPIN));
}

// ---- row 3: an account the wizard did not make

#[test]
fn a_foreign_account_marks_done() {
    let s = Sys::new();
    s.add_human("anaconda", 1000);
    s.prepare();
    assert_eq!(s.marker_state(), (true, true));
    assert_eq!(s.dropin(), None);
    assert_eq!(s.commands(), LOCK_CMDS);
}

// ---- row 4: verified, Finish not done, boots < 3

#[test]
fn verified_account_resumes_the_wizard() {
    let s = Sys::new();
    s.add_human("ada", 1000);
    s.state(&verified(1));
    s.prepare();
    assert_eq!(s.dropin().as_deref(), Some(DROPIN));
    assert_eq!(s.state_json()["boots"], 2);
    assert_eq!(s.state_json()["account"]["stage"], "verified");
    assert!(s.commands().is_empty());
}

// ---- row 5: verified, boots >= 3

#[test]
fn verified_account_after_three_boots_finishes_with_defaults() {
    let s = Sys::new();
    s.add_human("ada", 1000);
    s.write("etc/plasmalogin.conf.d/99-atlas-wizard.conf", DROPIN);
    s.state(&verified(3));
    let out = s.prepare();
    assert_eq!(s.marker_state(), (true, true));
    assert_eq!(s.dropin(), None);
    assert_eq!(s.commands(), LOCK_CMDS);
    assert_eq!(s.state_json()["finish"], "markers");
    let log = String::from_utf8_lossy(&out.stderr);
    assert!(
        log.contains("settings for the new user are skipped"),
        "{log}"
    );
}

#[test]
fn verified_account_whose_finish_began_is_finished() {
    let s = Sys::new();
    s.add_human("ada", 1000);
    s.state(
        r#"{"format":1,"boots":1,"account":{"name":"ada","uid":1000,"stage":"verified"},"finish":"markers"}"#,
    );
    s.prepare();
    assert_eq!(s.marker_state(), (true, true));
}

// ---- row 6: fallback

#[test]
fn gave_up_starts_the_text_fallback() {
    let s = Sys::new();
    s.write("etc/plasmalogin.conf.d/99-atlas-wizard.conf", DROPIN);
    s.state(r#"{"format":1,"boots":1,"gave_up":true}"#);
    s.prepare();
    assert_eq!(s.dropin(), None, "no setup autologin in text mode");
    assert_eq!(s.commands(), [FALLBACK_CMD]);
    assert_eq!(s.marker_state(), (false, false));
}

#[test]
fn three_boots_without_an_account_start_the_fallback() {
    let s = Sys::new();
    s.state(r#"{"format":1,"boots":3}"#);
    s.prepare();
    assert_eq!(s.commands(), [FALLBACK_CMD]);
    assert_eq!(s.state_json()["boots"], 3, "not counted again");
}

#[test]
fn the_kernel_command_line_forces_the_fallback() {
    let s = Sys::new();
    s.cmdline("rhgb atlas.wizard=fallback");
    s.prepare();
    assert_eq!(s.commands(), [FALLBACK_CMD]);
    assert_eq!(s.dropin(), None);
}

#[test]
fn fallback_command_line_with_a_verified_account_finishes_with_defaults() {
    for boots in [1, 3] {
        let s = Sys::new();
        s.cmdline("atlas.wizard=fallback");
        s.add_human("ada", 1000);
        s.state(&verified(boots));
        s.prepare();
        assert_eq!(s.marker_state(), (true, true), "boots {boots}");
        assert_eq!(s.dropin(), None);
        assert!(!s.commands().contains(&FALLBACK_CMD.to_string()));
    }
}

#[test]
fn a_locked_setup_user_is_unlocked_when_the_wizard_must_run() {
    let s = Sys::new();
    s.lock_atlas_setup();
    s.prepare();
    assert_eq!(s.dropin().as_deref(), Some(DROPIN));
    assert_eq!(
        s.commands(),
        [
            "/usr/bin/chage -E -1 atlas-setup",
            "/usr/sbin/usermod -s /bin/sh atlas-setup"
        ]
    );
    assert!(s.read("etc/passwd").contains(":/bin/sh\n"));
    assert!(
        !s.read("etc/shadow")
            .contains("atlas-setup:!*:19000:0:99999:7::0:")
    );
    s.prepare();
    assert_eq!(s.commands().len(), 2, "a healthy boot runs nothing");
}

#[test]
fn a_healthy_setup_user_runs_no_command_when_the_wizard_runs() {
    let s = Sys::new();
    s.prepare();
    assert_eq!(s.dropin().as_deref(), Some(DROPIN));
    assert!(s.commands().is_empty());
}

// ---- row 7: otherwise

#[test]
fn first_boot_writes_the_autologin_and_counts() {
    let s = Sys::new();
    s.prepare();
    assert_eq!(s.dropin().as_deref(), Some(DROPIN));
    assert_eq!(s.state_json()["boots"], 1);
    assert!(s.commands().is_empty());
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(s.path("etc/plasmalogin.conf.d/99-atlas-wizard.conf"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777
    };
    assert_eq!(mode, 0o644);
    assert!(
        fs::metadata(s.path("var/lib/atlas-wizard/state.json"))
            .unwrap()
            .len()
            > 0
    );
}

#[test]
fn wizard_runs_three_boots_then_the_fallback_takes_over() {
    let s = Sys::new();
    for n in 1..=3 {
        s.prepare();
        assert_eq!(s.state_json()["boots"], n);
        assert_eq!(s.dropin().as_deref(), Some(DROPIN), "boot {n}");
    }
    s.prepare();
    assert_eq!(s.dropin(), None, "the fourth boot is text mode");
    assert_eq!(s.commands(), [FALLBACK_CMD]);
}

// ---- power cuts and damaged files

#[test]
fn state_file_missing_is_a_first_boot() {
    let s = Sys::new();
    assert!(!s.exists("var/lib/atlas-wizard/state.json"));
    s.prepare();
    assert_eq!(s.state_json()["boots"], 1);
}

#[test]
fn truncated_state_is_moved_aside_and_treated_as_empty() {
    let s = Sys::new();
    s.state(r#"{"format":1,"boots":2,"acc"#);
    let out = s.prepare();
    assert_eq!(s.state_json()["boots"], 1, "treated as empty");
    assert_eq!(s.dropin().as_deref(), Some(DROPIN));
    let files = s.files_under("var/lib/atlas-wizard");
    assert!(
        files.iter().any(|f| f.starts_with("state.json.bad-")),
        "{files:?}"
    );
    let log = String::from_utf8_lossy(&out.stderr);
    assert!(log.contains("unparsable"), "{log}");
}

#[test]
fn a_leftover_temp_file_from_a_cut_write_is_ignored() {
    let s = Sys::new();
    s.state(r#"{"format":1,"boots":1}"#);
    s.write(
        "var/lib/atlas-wizard/.state.json.tmp-123-0",
        r#"{"format":1,"boots":99,"gave_"#,
    );
    s.prepare();
    assert_eq!(s.state_json()["boots"], 2);
    // And with no state file at all, only the temp file.
    let t = Sys::new();
    t.write("var/lib/atlas-wizard/.state.json.tmp-123-0", "{");
    t.prepare();
    assert_eq!(t.state_json()["boots"], 1);
    assert_eq!(t.dropin().as_deref(), Some(DROPIN));
}

#[test]
fn a_cut_after_the_state_was_saved_still_ends_with_the_autologin() {
    // Boot 1 saved boots=1 and lost power before writing the drop-in.
    let s = Sys::new();
    s.state(r#"{"format":1,"boots":1}"#);
    assert_eq!(s.dropin(), None);
    s.prepare();
    assert_eq!(s.dropin().as_deref(), Some(DROPIN));
    assert_eq!(s.state_json()["boots"], 2);
}

#[test]
fn a_half_made_account_is_left_to_the_helper() {
    // Stage `created` and no hash yet: not a human account, not verified.
    let s = Sys::new();
    s.write(
        "etc/passwd",
        &format!(
            "{}ada:x:1000:1000:Ada:/home/ada:/bin/bash\n",
            s.read("etc/passwd")
        ),
    );
    s.write(
        "etc/shadow",
        &format!("{}ada:!:19000::::::\n", s.read("etc/shadow")),
    );
    s.state(r#"{"format":1,"boots":1,"account":{"name":"ada","uid":1000,"stage":"created"}}"#);
    s.prepare();
    assert_eq!(s.dropin().as_deref(), Some(DROPIN));
    assert_eq!(s.state_json()["boots"], 2);
    assert_eq!(s.state_json()["account"]["stage"], "created", "kept");
    assert!(s.commands().is_empty(), "prepare never deletes accounts");
    assert_eq!(s.marker_state(), (false, false));
}

#[test]
fn an_unknown_stage_from_a_newer_wizard_is_kept() {
    if !is_root() {
        return; // the home must be owned by uid 1000
    }
    let s = Sys::new();
    s.add_human("ada", 1000);
    s.state(r#"{"format":2,"boots":1,"future":{"a":1},"account":{"name":"ada","uid":1000,"stage":"fingerprint-set"}}"#);
    s.prepare();
    let j = s.state_json();
    assert_eq!(j["boots"], 2, "verifies, so the wizard resumes");
    assert_eq!(j["account"]["stage"], "fingerprint-set");
    assert_eq!(j["future"]["a"], 1);
}

#[test]
fn an_unreadable_state_means_text_mode_not_a_loop() {
    let s = Sys::new();
    // A directory where the file belongs: reading it fails.
    fs::create_dir_all(s.path("var/lib/atlas-wizard/state.json")).unwrap();
    s.prepare();
    assert_eq!(s.commands(), [FALLBACK_CMD]);
    assert_eq!(s.dropin(), None);
}

#[test]
fn a_dropin_that_cannot_be_written_means_text_mode() {
    let s = Sys::new();
    s.write("etc/plasmalogin.conf.d", "a file, not a directory");
    s.prepare();
    assert_eq!(s.commands(), [FALLBACK_CMD]);
}

#[test]
fn failing_commands_never_fail_the_boot() {
    let s = Sys::new();
    s.markers();
    let out = s
        .command("prepare")
        .env("ATLAS_WIZARD_TEST_FAIL", "chage,usermod,systemctl,loginctl")
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(s.commands(), LOCK_CMDS, "both were tried");
    let log = String::from_utf8_lossy(&out.stderr);
    assert!(log.contains("fake failure"), "{log}");
}

#[test]
fn a_failing_fallback_start_does_not_fail_the_boot() {
    let s = Sys::new();
    s.state(r#"{"format":1,"gave_up":true}"#);
    let out = s
        .command("prepare")
        .env("ATLAS_WIZARD_TEST_FAIL", "systemctl")
        .output()
        .unwrap();
    assert!(out.status.success());
}

#[test]
fn passwd_that_cannot_be_read_changes_nothing() {
    let s = Sys::new();
    // A directory where passwd belongs: not "no accounts".
    fs::remove_file(s.path("etc/passwd")).unwrap();
    fs::create_dir(s.path("etc/passwd")).unwrap();
    s.write("etc/plasmalogin.conf.d/99-atlas-wizard.conf", DROPIN);
    s.prepare();
    assert_eq!(s.dropin().as_deref(), Some(DROPIN), "left as it was");
    assert!(!s.exists("var"));
    assert_eq!(s.marker_state(), (false, false));
}

#[test]
fn the_logs_say_what_was_decided() {
    let s = Sys::new();
    let out = s.prepare();
    let log = String::from_utf8_lossy(&out.stderr);
    assert!(log.contains("decision: RunWizard"), "{log}");
}

#[test]
fn done_path_timing() {
    // Not a pass/fail on speed (the CI box varies): a done boot is a few
    // stats, so 100 runs must finish well inside a generous bound, and the
    // number is printed for the report (cargo test -- --nocapture).
    let s = Sys::new();
    s.markers();
    s.prepare();
    let start = Instant::now();
    for _ in 0..100 {
        assert!(s.command("prepare").output().unwrap().status.success());
    }
    let each = start.elapsed() / 100;
    eprintln!("done-path prepare: {each:?} per run (100 runs, incl. process start)");
    assert!(each.as_millis() < 100, "{each:?}");
}
