//! The boot decision (DESIGN.md, State and recovery): a pure function from
//! what `prepare` found to what it does, plus the state after it.

use crate::accounts::Human;
use crate::markers::Present;
use crate::state::{FINISH_MARKERS, Stage, State};

/// The wizard is started at most this many boots before giving way.
pub const MAX_BOOTS: u32 = 3;

/// The kernel command line's `atlas.wizard=` value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Cmdline {
    /// Not given, or a value we do not know.
    #[default]
    None,
    /// `atlas.wizard=skip`
    Skip,
    /// `atlas.wizard=fallback`
    Fallback,
}

/// Parses `/proc/cmdline` text. Words are split on whitespace; the last
/// `atlas.wizard=` wins; unknown values count as not given.
pub fn parse_cmdline(cmdline: &str) -> Cmdline {
    let mut out = Cmdline::None;
    for word in cmdline.split_whitespace() {
        if let Some(v) = word.strip_prefix("atlas.wizard=") {
            out = match v {
                "skip" => Cmdline::Skip,
                "fallback" => Cmdline::Fallback,
                _ => Cmdline::None,
            };
        }
    }
    out
}

/// Everything the decision depends on.
#[derive(Debug, Clone, Default)]
pub struct BootInput {
    /// Which done markers exist.
    pub markers: Present,
    /// The `atlas.wizard=` value.
    pub cmdline: Cmdline,
    /// Human accounts found in passwd and shadow.
    pub humans: Vec<Human>,
    /// The loaded state.
    pub state: State,
}

/// What `prepare` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootAction {
    /// Setup is done: remove the autologin drop-in, lock `atlas-setup`,
    /// write the missing marker (when `write_missing_marker`).
    Cleanup {
        /// Only one of the two markers exists.
        write_missing_marker: bool,
    },
    /// An account exists that the wizard did not make: write both markers,
    /// then clean up.
    MarkDoneAndCleanup,
    /// Write the setup autologin; start the wizard.
    RunWizard {
        /// The account is verified: resume after the Account page.
        resume_after_account: bool,
    },
    /// Finish with defaults (markers, cleanup); login screen.
    FinishWithDefaults,
    /// Start the text-mode fallback; no setup autologin.
    Fallback,
}

/// True when the state's account is verified *and* is a human account found
/// in passwd and shadow with the same name and uid. A verified account that
/// is missing or lacks a hash is half-made, so this is false for it.
fn verified_and_present(input: &BootInput) -> bool {
    input.state.account.as_ref().is_some_and(|a| {
        a.stage == Stage::Verified
            && input
                .humans
                .iter()
                .any(|h| h.name == a.name && h.uid == a.uid)
    })
}

/// A human account the state does not name.
fn foreign_human(input: &BootInput) -> bool {
    input.humans.iter().any(|h| {
        !input
            .state
            .account
            .as_ref()
            .is_some_and(|a| a.name == h.name && a.uid == h.uid)
    })
}

/// The decision table, in the order of DESIGN.md.
pub fn decide(input: &BootInput) -> BootAction {
    let s = &input.state;
    // 1. a done marker
    if input.markers.any() {
        return BootAction::Cleanup {
            write_missing_marker: !input.markers.both(),
        };
    }
    // 2. atlas.wizard=skip and a usable human account
    if input.cmdline == Cmdline::Skip && !input.humans.is_empty() {
        return BootAction::MarkDoneAndCleanup;
    }
    // 3. a usable human account not in the state
    if foreign_human(input) {
        return BootAction::MarkDoneAndCleanup;
    }
    let verified = verified_and_present(input);
    // 4. verified, Finish not done, boots < 3
    if verified && s.finish.is_none() && s.boots < MAX_BOOTS {
        return BootAction::RunWizard {
            resume_after_account: true,
        };
    }
    // 5. verified, boots >= 3
    if verified && s.boots >= MAX_BOOTS {
        return BootAction::FinishWithDefaults;
    }
    // (not in the table) verified and Finish already began, but no marker
    // yet: Finish is idempotent, so complete it instead of re-running pages.
    if verified && s.finish.is_some() {
        return BootAction::FinishWithDefaults;
    }
    // 6. gave up, or boots >= 3 without a (verified) account, or forced
    if s.gave_up || s.boots >= MAX_BOOTS || input.cmdline == Cmdline::Fallback {
        return BootAction::Fallback;
    }
    // 7. otherwise
    BootAction::RunWizard {
        resume_after_account: false,
    }
}

/// The state to save after `action` (the caller saves it before acting, so a
/// crash mid-way still counts the boot).
pub fn next_state(state: &State, action: &BootAction) -> State {
    let mut next = state.clone();
    match action {
        BootAction::RunWizard { .. } => next.boots = next.boots.saturating_add(1),
        BootAction::FinishWithDefaults => {
            next.finish = Some(FINISH_MARKERS.to_string());
        }
        BootAction::Cleanup { .. } | BootAction::MarkDoneAndCleanup | BootAction::Fallback => {}
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::Account;

    fn human(name: &str, uid: u32) -> Human {
        Human {
            name: name.into(),
            uid,
        }
    }

    fn acct(stage: Stage) -> Option<Account> {
        Some(Account {
            name: "ada".into(),
            uid: 1000,
            stage,
        })
    }

    fn input(boots: u32, account: Option<Account>, humans: Vec<Human>) -> BootInput {
        BootInput {
            state: State {
                boots,
                account,
                ..State::default()
            },
            humans,
            ..BootInput::default()
        }
    }

    const RUN: BootAction = BootAction::RunWizard {
        resume_after_account: false,
    };
    const RESUME: BootAction = BootAction::RunWizard {
        resume_after_account: true,
    };

    // Row 1
    #[test]
    fn row1_done_marker() {
        for (atlas, plasma, missing) in [
            (true, false, true),
            (false, true, true),
            (true, true, false),
        ] {
            let mut i = input(0, None, vec![]);
            i.markers = Present { atlas, plasma };
            assert_eq!(
                decide(&i),
                BootAction::Cleanup {
                    write_missing_marker: missing
                }
            );
        }
    }

    #[test]
    fn row1_beats_everything() {
        let mut i = input(9, acct(Stage::Verified), vec![human("zed", 1500)]);
        i.state.gave_up = true;
        i.cmdline = Cmdline::Fallback;
        i.markers = Present {
            atlas: false,
            plasma: true,
        };
        assert_eq!(
            decide(&i),
            BootAction::Cleanup {
                write_missing_marker: true
            }
        );
    }

    // Row 2
    #[test]
    fn row2_skip_with_human() {
        let mut i = input(0, None, vec![human("ada", 1000)]);
        i.cmdline = Cmdline::Skip;
        assert_eq!(decide(&i), BootAction::MarkDoneAndCleanup);
        // Even when the human is the state's own verified account.
        let mut i = input(1, acct(Stage::Verified), vec![human("ada", 1000)]);
        i.cmdline = Cmdline::Skip;
        assert_eq!(decide(&i), BootAction::MarkDoneAndCleanup);
    }

    #[test]
    fn row2_skip_without_human_runs_wizard() {
        let mut i = input(0, None, vec![]);
        i.cmdline = Cmdline::Skip;
        assert_eq!(decide(&i), RUN);
    }

    // Row 3
    #[test]
    fn row3_foreign_human() {
        let i = input(0, None, vec![human("kick", 1000)]);
        assert_eq!(decide(&i), BootAction::MarkDoneAndCleanup);
        // The state names a different account.
        let i = input(1, acct(Stage::Verified), vec![human("kick", 1001)]);
        assert_eq!(decide(&i), BootAction::MarkDoneAndCleanup);
        // Same name, different uid is not "in the state".
        let i = input(1, acct(Stage::Verified), vec![human("ada", 1001)]);
        assert_eq!(decide(&i), BootAction::MarkDoneAndCleanup);
    }

    // Row 4
    #[test]
    fn row4_verified_resume() {
        for boots in 0..3 {
            let i = input(boots, acct(Stage::Verified), vec![human("ada", 1000)]);
            assert_eq!(decide(&i), RESUME, "boots {boots}");
        }
    }

    // Row 5
    #[test]
    fn row5_verified_boots_exhausted() {
        for boots in [3, 4, 100] {
            let i = input(boots, acct(Stage::Verified), vec![human("ada", 1000)]);
            assert_eq!(decide(&i), BootAction::FinishWithDefaults, "boots {boots}");
        }
    }

    #[test]
    fn boots_exactly_three_is_the_edge() {
        let v = vec![human("ada", 1000)];
        assert_eq!(decide(&input(2, acct(Stage::Verified), v.clone())), RESUME);
        assert_eq!(
            decide(&input(3, acct(Stage::Verified), v)),
            BootAction::FinishWithDefaults
        );
        assert_eq!(decide(&input(2, None, vec![])), RUN);
        assert_eq!(decide(&input(3, None, vec![])), BootAction::Fallback);
    }

    #[test]
    fn verified_finish_begun_completes_instead_of_rerunning() {
        let mut i = input(1, acct(Stage::Verified), vec![human("ada", 1000)]);
        i.state.finish = Some("markers".into());
        assert_eq!(decide(&i), BootAction::FinishWithDefaults);
    }

    // Row 6
    #[test]
    fn row6_gave_up() {
        let mut i = input(1, None, vec![]);
        i.state.gave_up = true;
        assert_eq!(decide(&i), BootAction::Fallback);
    }

    #[test]
    fn row6_three_boots_no_account() {
        assert_eq!(decide(&input(3, None, vec![])), BootAction::Fallback);
    }

    #[test]
    fn row6_cmdline_fallback() {
        let mut i = input(0, None, vec![]);
        i.cmdline = Cmdline::Fallback;
        assert_eq!(decide(&i), BootAction::Fallback);
    }

    // Row 7
    #[test]
    fn row7_otherwise() {
        assert_eq!(decide(&input(0, None, vec![])), RUN);
        assert_eq!(decide(&BootInput::default()), RUN);
    }

    // Edges
    #[test]
    fn verified_without_hash_is_half_made() {
        // The shadow hash is missing, so ada is not in the human list.
        let i = input(1, acct(Stage::Verified), vec![]);
        assert_eq!(decide(&i), RUN);
        let i = input(3, acct(Stage::Verified), vec![]);
        assert_eq!(decide(&i), BootAction::Fallback);
        let mut i = input(0, acct(Stage::Verified), vec![]);
        i.state.gave_up = true;
        assert_eq!(decide(&i), BootAction::Fallback);
    }

    #[test]
    fn half_made_stages_are_not_resumed() {
        for st in [Stage::Creating, Stage::Created, Stage::PasswordSet] {
            assert_eq!(decide(&input(1, acct(st), vec![])), RUN, "{st:?}");
            assert_eq!(
                decide(&input(3, acct(st), vec![])),
                BootAction::Fallback,
                "{st:?}"
            );
        }
        // Password set means a hash exists, so the account may already be
        // listed; with the state naming it, it is still half-made, not foreign.
        let i = input(1, acct(Stage::PasswordSet), vec![human("ada", 1000)]);
        assert_eq!(decide(&i), RUN);
    }

    #[test]
    fn cmdline_parsing() {
        assert_eq!(parse_cmdline(""), Cmdline::None);
        assert_eq!(
            parse_cmdline("BOOT_IMAGE=/vmlinuz root=/dev/sda quiet\n"),
            Cmdline::None
        );
        assert_eq!(parse_cmdline("quiet atlas.wizard=skip rhgb"), Cmdline::Skip);
        assert_eq!(parse_cmdline("atlas.wizard=fallback\n"), Cmdline::Fallback);
        assert_eq!(parse_cmdline("atlas.wizard=other"), Cmdline::None);
        assert_eq!(
            parse_cmdline("atlas.wizard=skip atlas.wizard=fallback"),
            Cmdline::Fallback
        );
        assert_eq!(parse_cmdline("xatlas.wizard=skip"), Cmdline::None);
        assert_eq!(parse_cmdline("atlas.wizard=skipx"), Cmdline::None);
    }

    #[test]
    fn next_state_counts_boots_when_running() {
        let s = State {
            boots: 1,
            ..State::default()
        };
        assert_eq!(next_state(&s, &RUN).boots, 2);
        assert_eq!(next_state(&s, &RESUME).boots, 2);
        let max = State {
            boots: u32::MAX,
            ..State::default()
        };
        assert_eq!(next_state(&max, &RUN).boots, u32::MAX);
    }

    #[test]
    fn next_state_other_actions() {
        let mut s = State {
            boots: 3,
            ..State::default()
        };
        s.extra.insert("future".into(), serde_json::json!(1));
        for a in [
            BootAction::Cleanup {
                write_missing_marker: true,
            },
            BootAction::MarkDoneAndCleanup,
            BootAction::Fallback,
        ] {
            assert_eq!(next_state(&s, &a), s);
        }
        let f = next_state(&s, &BootAction::FinishWithDefaults);
        assert_eq!(f.finish.as_deref(), Some("markers"));
        assert_eq!(f.boots, 3);
        assert_eq!(f.extra, s.extra);
    }

    #[test]
    fn decide_is_pure() {
        let i = input(2, acct(Stage::Verified), vec![human("ada", 1000)]);
        assert_eq!(decide(&i), decide(&i));
    }
}
