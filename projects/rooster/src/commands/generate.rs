use clip::{copy_to_clipboard, paste_keys};
use generate::{check_password_len, PasswordSpec};
use io::{CliReader, CliWriter};
use io::{OutputType, Style};
use password;

use std::ops::Deref;

pub fn callback_exec(
    matches: &clap::ArgMatches,
    store: &mut password::v2::PasswordStore,
    _reader: &mut impl CliReader,
    writer: &mut impl CliWriter,
) -> Result<(), i32> {
    let app_name = matches.value_of("app").unwrap();
    let username = matches.value_of("username").unwrap();

    if store.has_password(app_name.deref()) {
        writer.writeln(
            Style::error("Woops, there is already an app with that name."),
            OutputType::Error,
        );
        return Err(1);
    }

    let pwspec = PasswordSpec::new(
        matches.is_present("alnum"),
        matches
            .value_of("length")
            .and_then(|len| check_password_len(len.parse::<usize>().ok(), writer)),
    );

    let password_as_string = match pwspec.generate_hard_password() {
        Ok(password_as_string) => password_as_string,
        Err(io_err) => {
            writer.writeln(
                Style::error(format!(
                    "Woops, I could not generate the password (reason: {:?}).",
                    io_err
                )),
                OutputType::Error,
            );
            return Err(1);
        }
    };

    // Read the master password and try to save the new password.
    let password_as_string_clipboard = password_as_string.clone();
    let password = password::v2::Password::new(app_name.clone(), username, password_as_string);

    match store.add_password(password) {
        Ok(_) => {
            if matches.is_present("show") {
                writer.writeln(
                    Style::success(format!(
                        "Alright! Here is your password: {}",
                        password_as_string_clipboard.deref()
                    )),
                    OutputType::Standard,
                );
                return Ok(());
            }

            if copy_to_clipboard(&password_as_string_clipboard).is_err() {
                writer.writeln(
                    Style::success(format!(
                        "Hmm, I tried to copy your new password to your clipboard, but \
                         something went wrong. Don't worry, it's saved, and you can see it \
                         with `rooster get {} --show`",
                        app_name
                    )),
                    OutputType::Standard,
                );
            } else {
                writer.writeln(
                    Style::success(format!(
                        "Alright! I've saved your new password. You can paste it anywhere with {}.",
                        paste_keys()
                    )),
                    OutputType::Standard,
                );
            }

            Ok(())
        }
        Err(err) => {
            writer.writeln(
                Style::error(format!(
                    "\nI couldn't add this password (reason: {:?}).",
                    err
                )),
                OutputType::Error,
            );
            Err(1)
        }
    }
}
