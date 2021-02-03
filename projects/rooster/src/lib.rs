// #![allow(useless_format, too_many_arguments)]

extern crate ansi_term;
extern crate serde_json;

use crate::io::{CliReader, CliWriter};
use crate::io::{OutputType, Style};
use crate::password::v2::PasswordStore;
use clap::{App, AppSettings, Arg, ArgMatches};
use rutil::SafeString;
use rutil::SafeVec;
use std::env;
use std::fs::File;
use std::io::Read;
use std::io::Result as IoResult;
use std::ops::Deref;
use std::path::{Path, PathBuf};

mod aes;
mod clip;
mod commands;
mod ffi;
mod generate;
pub mod io;
mod list;
mod password;
mod quale;

// We conditionally compile this module to avoid "unused function" warnings.
#[cfg(all(unix, not(target_os = "macos")))]
mod shell_escape;

fn validate_arg_digits(v: &str) -> Result<(), String> {
    if v.chars()
        .map(|c| char::is_ascii_digit(&c))
        .collect::<Vec<bool>>()
        .contains(&false)
    {
        return Err(String::from("The value must be made of digits"));
    }
    Ok(())
}

fn open_password_file(filename: &str) -> IoResult<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    options.write(true);
    options.create(false);
    options.open(&Path::new(filename))
}

fn create_password_file(filename: &str) -> IoResult<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    options.write(true);
    options.create(true);
    options.open(&Path::new(filename))
}

fn sync_password_store(
    store: &mut PasswordStore,
    file: &mut File,
    writer: &mut impl CliWriter,
) -> Result<(), i32> {
    if let Err(err) = store.sync(file) {
        writer.writeln(
            Style::error(format!(
                "I could not save the password file (reason: {:?}).",
                err
            )),
            OutputType::Error,
        );
        return Err(1);
    }

    return Ok(());
}

fn get_password_store(
    file: &mut File,
    reader: &mut impl CliReader,
    writer: &mut impl CliWriter,
) -> Result<password::v2::PasswordStore, i32> {
    // Read the Rooster file contents.
    let mut input: SafeVec = SafeVec::new(Vec::new());
    file.read_to_end(input.inner_mut()).map_err(|_| 1)?;

    return get_password_store_from_input_interactive(&input, 3, false, false, reader, writer)
        .map_err(|_| 1);
}

fn get_password_store_from_input_interactive(
    input: &SafeVec,
    retries: i32,
    force_upgrade: bool,
    retry: bool,
    reader: &mut impl CliReader,
    writer: &mut impl CliWriter,
) -> Result<password::v2::PasswordStore, password::PasswordError> {
    if retries == 0 {
        writer.writeln(
            Style::error(
                "Decryption of your Rooster file keeps failing. \
             Your Rooster file is probably corrupted.",
            ),
            OutputType::Error,
        );
        return Err(password::PasswordError::CorruptionLikelyError);
    }

    if retry {
        writer.writeln(
            Style::error("Woops, that's not the right password. Let's try again."),
            OutputType::Error,
        );
    }

    let master_password = match ask_master_password(reader, writer) {
        Ok(p) => p,
        Err(err) => {
            writer.writeln(
                Style::error(format!(
                    "Woops, I could not read your master password (reason: {}).",
                    err
                )),
                OutputType::Error,
            );
            return Err(password::PasswordError::Io(err));
        }
    };

    match get_password_store_from_input(&input, &master_password, force_upgrade) {
        Ok(store) => {
            return Ok(store);
        }
        Err(password::PasswordError::CorruptionError) => {
            writer.writeln(
                Style::error("Your Rooster file is corrupted."),
                OutputType::Error,
            );
            return Err(password::PasswordError::CorruptionError);
        }
        Err(password::PasswordError::OutdatedRoosterBinaryError) => {
            writer.writeln(
                Style::error("I could not open the Rooster file because your version of Rooster is outdated."),
                OutputType::Error,
            );
            writer.writeln(
                Style::error("Try upgrading Rooster to the latest version."),
                OutputType::Error,
            );
            return Err(password::PasswordError::OutdatedRoosterBinaryError);
        }
        Err(password::PasswordError::Io(err)) => {
            writer.writeln(
                Style::error(format!(
                    "I couldn't open your Rooster file (reason: {:?})",
                    err
                )),
                OutputType::Error,
            );
            return Err(password::PasswordError::Io(err));
        }
        Err(password::PasswordError::NeedUpgradeErrorFromV1) => {
            writer.writeln(
                Style::error("Your Rooster file has version 1. You need to upgrade to version 2.\n\nWARNING: If in doubt, it could mean you've been hacked. Only \
                 proceed if you recently upgraded your Rooster installation.\nUpgrade to version 2? [y/n]"),
                OutputType::Error
            );
            loop {
                match reader.read_line() {
                    Ok(line) => {
                        if line.starts_with('y') {
                            // This time we'll try to upgrade
                            return get_password_store_from_input_interactive(
                                &input, retries, true, false, reader, writer,
                            );
                        } else if line.starts_with('n') {
                            // The user doesn't want to upgrade, that's fine
                            return Err(password::PasswordError::NoUpgradeError);
                        } else {
                            writer.writeln(
                                Style::error("I did not get that. Upgrade from v1 to v2? [y/n]"),
                                OutputType::Error,
                            );
                        }
                    }
                    Err(io_err) => {
                        writer.writeln(
                            Style::error(format!(
                                "Woops, an error occured while reading your response (reason: {:?}).",
                                io_err
                            )),
                            OutputType::Error,
                        );
                        return Err(password::PasswordError::Io(io_err));
                    }
                }
            }
        }
        _ => {
            return get_password_store_from_input_interactive(
                &input,
                retries - 1,
                false,
                true,
                reader,
                writer,
            );
        }
    }
}

fn get_password_store_from_input(
    input: &SafeVec,
    master_password: &SafeString,
    upgrade: bool,
) -> Result<password::v2::PasswordStore, password::PasswordError> {
    // Try to open the file as is.
    match password::v2::PasswordStore::from_input(master_password.clone(), input.clone()) {
        Ok(store) => {
            return Ok(store);
        }
        Err(password::PasswordError::CorruptionError) => {
            return Err(password::PasswordError::CorruptionError);
        }
        Err(password::PasswordError::OutdatedRoosterBinaryError) => {
            return Err(password::PasswordError::OutdatedRoosterBinaryError);
        }
        Err(password::PasswordError::NeedUpgradeErrorFromV1) => {
            if !upgrade {
                return Err(password::PasswordError::NeedUpgradeErrorFromV1);
            }

            // If we can't open the file, we may need to upgrade its format first.
            match password::upgrade(master_password.clone(), input.clone()) {
                Ok(store) => {
                    return Ok(store);
                }
                Err(err) => {
                    return Err(err);
                }
            }
        }
        Err(err) => {
            return Err(err);
        }
    }
}

fn ask_master_password(
    reader: &mut impl CliReader,
    writer: &mut impl CliWriter,
) -> IoResult<SafeString> {
    writer.write("Type your master password: ", OutputType::Tty);
    reader.read_password()
}

pub fn main_with_args(
    args: &[&str],
    reader: &mut impl CliReader,
    writer: &mut impl CliWriter,
    rooster_file_path: &PathBuf,
) -> i32 {
    let matches: ArgMatches = App::new("rooster")
        .global_setting(AppSettings::HelpRequired)
        .global_setting(AppSettings::DisableHelpSubcommand)
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .about("Welcome to Rooster, the simple password manager for geeks :-)")
        .version(env!("CARGO_PKG_VERSION"))
        .subcommand(
            App::new("init").about("Create a new password file").arg(
                Arg::new("force-for-tests")
                    .long("force-for-tests")
                    .hidden(true)
                    .about("Forces initializing the file, used in integration tests only"),
            ),
        )
        .subcommand(
            App::new("add")
                .about("Add a new password manually")
                .arg(
                    Arg::new("app")
                        .required(true)
                        .about("The name of the app (fuzzy-matched)"),
                )
                .arg(
                    Arg::new("username")
                        .required(true)
                        .about("Your username for this account"),
                )
                .arg(
                    Arg::new("show")
                        .short('s')
                        .long("show")
                        .about("Show the password instead of copying it to the clipboard"),
                ),
        )
        .subcommand(
            App::new("change")
                .about("Change a password manually")
                .arg(
                    Arg::new("app")
                        .required(true)
                        .about("The name of the app (fuzzy-matched)"),
                )
                .arg(
                    Arg::new("show")
                        .short('s')
                        .long("show")
                        .about("Show the password instead of copying it to the clipboard"),
                ),
        )
        .subcommand(
            App::new("delete").about("Delete a password").arg(
                Arg::new("app")
                    .required(true)
                    .about("The name of the app (fuzzy-matched)"),
            ),
        )
        .subcommand(
            App::new("generate")
                .about("Generate a password")
                .arg(
                    Arg::new("app")
                        .required(true)
                        .about("The name of the app (fuzzy-matched)"),
                )
                .arg(
                    Arg::new("username")
                        .required(true)
                        .about("Your username for this account"),
                )
                .arg(
                    Arg::new("show")
                        .short('s')
                        .long("show")
                        .about("Show the password instead of copying it to the clipboard"),
                )
                .arg(
                    Arg::new("alnum")
                        .short('a')
                        .long("alnum")
                        .about("Only use alpha numeric (a-z, A-Z, 0-9) in generated passwords"),
                )
                .arg(
                    Arg::new("length")
                        .short('l')
                        .long("length")
                        .default_value("32")
                        .about("Set a custom length for the generated password")
                        .validator(validate_arg_digits),
                ),
        )
        .subcommand(
            App::new("regenerate")
                .about("Regenerate a previously existing password")
                .arg(
                    Arg::new("app")
                        .required(true)
                        .about("The name of the app (fuzzy-matched)"),
                )
                .arg(
                    Arg::new("show")
                        .short('s')
                        .long("show")
                        .about("Show the password instead of copying it to the clipboard"),
                )
                .arg(
                    Arg::new("alnum")
                        .short('a')
                        .long("alnum")
                        .about("Only use alpha numeric (a-z, A-Z, 0-9) in generated passwords"),
                )
                .arg(
                    Arg::new("length")
                        .short('l')
                        .long("length")
                        .default_value("32")
                        .about("Set a custom length for the generated password")
                        .validator(validate_arg_digits),
                ),
        )
        .subcommand(
            App::new("get")
                .about("Retrieve a password")
                .arg(
                    Arg::new("app")
                        .required(true)
                        .about("The name of the app (fuzzy-matched)"),
                )
                .arg(
                    Arg::new("show")
                        .short('s')
                        .long("show")
                        .about("Show the password instead of copying it to the clipboard"),
                ),
        )
        .subcommand(
            App::new("rename")
                .about("Rename the app for a password")
                .arg(
                    Arg::new("app")
                        .required(true)
                        .about("The current name of the app (fuzzy-matched)"),
                )
                .arg(
                    Arg::new("new_name")
                        .required(true)
                        .about("The new name of the app"),
                ),
        )
        .subcommand(
            App::new("transfer")
                .about("Change the username for a password")
                .arg(
                    Arg::new("app")
                        .required(true)
                        .about("The current name of the app (fuzzy-matched)"),
                )
                .arg(
                    Arg::new("new_username")
                        .required(true)
                        .about("Your new username for this account"),
                ),
        )
        .subcommand(App::new("list").about("List all apps and usernames"))
        .subcommand(
            App::new("import")
                .setting(AppSettings::SubcommandRequiredElseHelp)
                .about("Import all your existing passwords from elsewhere")
                .subcommand(
                    App::new("json")
                        .about("Import a file generated with `rooster export json`")
                        .arg(
                            Arg::new("path")
                                .required(true)
                                .about("The path to the file you want to import"),
                        ),
                )
                .subcommand(
                    App::new("csv")
                        .about("Import a file generated with `rooster export csv`")
                        .arg(
                            Arg::new("path")
                                .required(true)
                                .about("The path to the file you want to import"),
                        ),
                )
                .subcommand(
                    App::new("1password")
                        .about("Import a \"Common Fields\" CSV export from 1Password")
                        .arg(
                            Arg::new("path")
                                .required(true)
                                .about("The path to the file you want to import"),
                        ),
                ),
        )
        .subcommand(
            App::new("export")
                .setting(AppSettings::SubcommandRequiredElseHelp)
                .about("Export raw password data")
                .subcommand(App::new("json").about("Export raw password data in JSON format"))
                .subcommand(App::new("csv").about("Export raw password data in CSV format"))
                .subcommand(
                    App::new("1password")
                        .about("Export raw password data in 1Password compatible CSV format"),
                ),
        )
        .subcommand(App::new("set-master-password").about("Set your master password"))
        .subcommand(
            App::new("set-scrypt-params")
                .about("Set the key derivation parameters")
                .arg(
                    Arg::new("log2n")
                        .required(true)
                        .about("The log2n parameter")
                        .validator(validate_arg_digits),
                )
                .arg(
                    Arg::new("r")
                        .required(true)
                        .about("The r parameter")
                        .validator(validate_arg_digits),
                )
                .arg(
                    Arg::new("p")
                        .required(true)
                        .about("The p parameter")
                        .validator(validate_arg_digits),
                )
                .arg(
                    Arg::new("force")
                        .short('f')
                        .long("force")
                        .about("Disable parameter checks"),
                ),
        )
        .get_matches_from(args);

    let subcommand = matches.subcommand_name().unwrap();

    let command_matches = matches.subcommand_matches(subcommand).unwrap();

    if subcommand == "init" {
        match commands::init::callback_exec(command_matches, reader, writer, rooster_file_path) {
            Err(i) => return i,
            _ => return 0,
        }
    }

    let password_file_path_as_string = rooster_file_path.to_string_lossy().into_owned();

    if !rooster_file_path.exists() {
        writer.writeln(Style::title("First time user"), OutputType::Standard);
        writer.nl(OutputType::Standard);
        writer.writeln(Style::info("Try `rooster init`."), OutputType::Standard);
        writer.nl(OutputType::Standard);
        writer.writeln(Style::title("Long time user"), OutputType::Standard);
        writer.nl(OutputType::Standard);
        writer.writeln(
            Style::info("Set the ROOSTER_FILE environment variable. For instance:"),
            OutputType::Standard,
        );
        writer.writeln(
            Style::info("    export ROOSTER_FILE=path/to/passwords.rooster"),
            OutputType::Standard,
        );
        return 1;
    }

    let mut file = match open_password_file(password_file_path_as_string.deref()) {
        Ok(file) => file,
        Err(err) => {
            match err.kind() {
                std::io::ErrorKind::NotFound => {
                    writer.writeln(
                        Style::error("Woops, I can't find your password file. Run `rooster init` to create one."),
                        OutputType::Error,
                    );
                }
                _ => {
                    writer.writeln(
                        Style::error(format!(
                            "Woops, I couldn't read your password file ({} for \"{}\").",
                            err, password_file_path_as_string
                        )),
                        OutputType::Error,
                    );
                }
            }
            return 1;
        }
    };

    let mut store = match get_password_store(&mut file, reader, writer) {
        Err(code) => return code,
        Ok(store) => store,
    };

    let callback = match subcommand {
        "get" => commands::get::callback_exec,
        "add" => commands::add::callback_exec,
        "delete" => commands::delete::callback_exec,
        "generate" => commands::generate::callback_exec,
        "regenerate" => commands::regenerate::callback_exec,
        "list" => commands::list::callback_exec,
        "import" => commands::import::callback_exec,
        "export" => commands::export::callback_exec,
        "set-master-password" => commands::set_master_password::callback_exec,
        "set-scrypt-params" => commands::set_scrypt_params::callback_exec,
        "rename" => commands::rename::callback_exec,
        "transfer" => commands::transfer::callback_exec,
        "change" => commands::change::callback_exec,
        _ => unreachable!("Validation should have been done by `clap` before"),
    };

    if let Err(code) = callback(command_matches, &mut store, reader, writer) {
        return code;
    }

    if let Err(code) = sync_password_store(&mut store, &mut file, writer) {
        return code;
    }

    return 0;
}
