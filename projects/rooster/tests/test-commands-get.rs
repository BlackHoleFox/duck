mod helpers;

use crate::helpers::prelude::*;

#[test]
fn test_command_get() {
    let rooster_file = tempfile();
    assert_eq!(
        0,
        main_with_args(
            &["rooster", "init", "--force-for-tests"],
            &mut CursorInput::new("\nxxxx\n"),
            &mut CursorOutput::new(),
            &rooster_file
        )
    );

    assert_eq!(
        0,
        main_with_args(
            &["rooster", "add", "-s", "First Website", "first@example.com"],
            &mut CursorInput::new("xxxx\nabcd\n"),
            &mut CursorOutput::new(),
            &rooster_file
        )
    );
    assert_eq!(
        0,
        main_with_args(
            &[
                "rooster",
                "add",
                "-s",
                "Second Website",
                "second@example.com"
            ],
            &mut CursorInput::new("xxxx\nefgh\n"),
            &mut CursorOutput::new(),
            &rooster_file
        )
    );

    // Checking fuzzy-search and password selection
    let mut output = CursorOutput::new();
    assert_eq!(
        0,
        main_with_args(
            &["rooster", "get", "-s", "wbst"],
            &mut CursorInput::new("xxxx\n1\n"),
            &mut output,
            &rooster_file
        )
    );
    let output_as_vecu8 = output.standard_cursor.into_inner();
    let output_as_string = String::from_utf8_lossy(output_as_vecu8.as_slice());
    assert!(output_as_string.contains("abcd"));
    assert!(output_as_string.contains("first@example.com"));

    // Checking fuzzy-search and password selection
    let mut output = CursorOutput::new();
    assert_eq!(
        0,
        main_with_args(
            &["rooster", "get", "-s", "wbst"],
            &mut CursorInput::new("xxxx\n2\n"),
            &mut output,
            &rooster_file
        )
    );
    let output_as_vecu8 = output.standard_cursor.into_inner();
    let output_as_string = String::from_utf8_lossy(output_as_vecu8.as_slice());
    assert!(output_as_string.contains("efgh"));
    assert!(output_as_string.contains("second@example.com"));
}
