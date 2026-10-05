//! Every string the text-mode fallback shows, in one place (English only in
//! 0.1.0). Nothing here is ever a password.

use wizard_core::password::{HashError, PasswordError, QualityIssue};
use wizard_core::validate::{FullNameError, NameError};

pub const INTRO: &str = "\nWelcome to AtlasOS.\n\
The graphical setup could not start. Create your account here.\n\
(Press Ctrl+C at any question to start over.)\n";
pub const ASK_FULL_NAME: &str = "Full name: ";
pub const ASK_USER_NAME: &str = "User name";
pub const ASK_PASSWORD: &str = "Password: ";
pub const ASK_PASSWORD_AGAIN: &str = "Password again: ";
pub const PASSWORDS_DIFFER: &str = "The two passwords are not the same. Try again.";
pub const NAME_NOT_TEXT: &str = "That is not valid text. Try again.";
pub const STARTING_OVER: &str = "Starting over.";
pub const CREATING: &str = "Creating the account...";
pub const DONE: &str = "Your account is ready. Starting the login screen...";
pub const ALREADY_SET_UP: &str = "This computer is already set up. Starting the login screen...";
pub const WAITING_FOR_INPUT: &str = "No input. Waiting...";
pub const NO_ECHO_OFF: &str =
    "The password cannot be typed safely here (the terminal would show it). Trying again...";

pub fn create_failed(reason: &str) -> String {
    format!("The account could not be created: {reason}\nLet's try again.")
}

pub const REASON_HASH: &str = "the password could not be prepared";
pub const REASON_USERADD: &str = "the system refused to add the user";
pub const REASON_CHPASSWD: &str = "the password could not be set";
pub const REASON_VERIFY: &str = "the new account did not pass its checks";
pub const REASON_STATE: &str = "the progress could not be saved";

pub fn user_name_error(e: NameError) -> &'static str {
    match e {
        NameError::Empty => "Enter a user name.",
        NameError::TooLong => "The user name can be at most 32 characters.",
        NameError::BadStart => "The user name must start with a lowercase letter or an underscore.",
        NameError::BadChar => {
            "The user name can only have lowercase letters, digits, underscores and hyphens."
        }
        NameError::Reserved => "That user name is reserved for the system. Choose another.",
        NameError::Exists => "That user name is already taken. Choose another.",
        NameError::Unreadable => "The list of users could not be read. Try again.",
    }
}

pub fn full_name_error(e: FullNameError) -> &'static str {
    match e {
        FullNameError::TooLong => "The name can be at most 255 bytes.",
        FullNameError::BadChar => "The name cannot have a colon, a comma or an equals sign.",
        FullNameError::Control => "The name cannot have control or invisible characters.",
    }
}

pub const FULL_NAME_EMPTY: &str = "Enter your name.";

pub fn password_error(e: PasswordError) -> &'static str {
    match e {
        PasswordError::TooShort => "The password must have at least 8 characters.",
        PasswordError::TooLong => "The password is too long (at most 511 bytes).",
        PasswordError::NotText => "The password must be text.",
        PasswordError::HasNul => "The password cannot have a null character.",
        PasswordError::ContainsUserName => "The password cannot contain the user name.",
        PasswordError::ContainsFullName => "The password cannot contain your name.",
        PasswordError::Quality(q) => match q {
            QualityIssue::UserRelated => "The password is too close to your name.",
            QualityIssue::Dictionary => "The password is a common word. Choose another.",
            QualityIssue::TooSimple => {
                "The password is too simple. Mix letters, digits and symbols."
            }
            QualityIssue::Repetitive => "The password repeats itself too much.",
            QualityIssue::TooShort => "The password is too short.",
            QualityIssue::Other => "The password is too weak. Choose another.",
        },
        PasswordError::Unavailable => "The password could not be checked. Try again.",
    }
}

pub fn hash_error(_e: HashError) -> &'static str {
    REASON_HASH
}
