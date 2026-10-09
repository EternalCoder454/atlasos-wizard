//! Test helper of `tests/container/real-tools.sh`, never installed: it lets the
//! shell script check wizard-core against the real shadow-utils, inside an
//! unprivileged container and against a throw-away root only.
//!
//! ```text
//! real-tools-helper hash                      password on stdin -> yescrypt hash
//! real-tools-helper user-name NAME            exit 0 when wizard-core accepts NAME
//! real-tools-helper full-name TEXT            exit 0 when it accepts TEXT; prints it trimmed
//! real-tools-helper secure-home ROOT NAME UID makes the home private; prints changed|unchanged
//! real-tools-helper verify ROOT NAME UID      exit 0 when `accounts::verify` passes; prints the code
//! real-tools-helper humans ROOT               lists the human accounts, one `name uid` a line
//! ```

use std::io::Read;
use std::path::Path;
use std::process::ExitCode;
use wizard_core::{accounts, password, validate};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        ["hash"] => {
            let mut pw = Vec::new();
            if std::io::stdin().read_to_end(&mut pw).is_err() {
                return ExitCode::from(2);
            }
            if pw.last() == Some(&b'\n') {
                pw.pop();
            }
            match password::hash(&pw) {
                Ok(h) => {
                    println!("{h}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e.code());
                    ExitCode::FAILURE
                }
            }
        }
        ["user-name", name] => match validate::user_name(name) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{}", e.code());
                ExitCode::FAILURE
            }
        },
        ["full-name", text] => match validate::full_name(text) {
            Ok(t) => {
                println!("{t}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{}", e.code());
                ExitCode::FAILURE
            }
        },
        ["secure-home", root, name, uid] => {
            let Ok(uid) = uid.parse() else {
                return ExitCode::from(2);
            };
            match accounts::secure_home(Path::new(root), name, uid) {
                Ok(changed) => {
                    println!("{}", if changed { "changed" } else { "unchanged" });
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{}", e.code());
                    ExitCode::FAILURE
                }
            }
        }
        ["verify", root, name, uid] => {
            let Ok(uid) = uid.parse() else {
                return ExitCode::from(2);
            };
            match accounts::verify(Path::new(root), name, uid) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    println!("{}", e.code());
                    ExitCode::FAILURE
                }
            }
        }
        ["humans", root] => match accounts::human_accounts(Path::new(root)) {
            Ok(list) => {
                for h in list {
                    println!("{} {}", h.name, h.uid);
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprintln!("usage: see the top of crates/wizard-core/examples/real-tools-helper.rs");
            ExitCode::from(2)
        }
    }
}
