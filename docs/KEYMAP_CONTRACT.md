# Spectrum keyboard input contract

Host event names and layouts stay at each frontend boundary; all frontends must produce the same Spectrum chord or joystick action for the same intended US-layout input.

| Host input | Spectrum action |
| --- | --- |
| Letter or digit without Shift | Corresponding matrix key |
| Letter with Shift | Caps Shift + letter |
| Digit with Shift (`! @ # $ % ^ & * ( )`) | Symbol Shift + corresponding digit |
| Spectrum punctuation (apostrophe, quote, semicolon, colon, comma, angle brackets, period, slash, question mark, hyphen, underscore, equals, plus, brackets, braces, backslash, vertical bar, backtick, tilde) | Symbol Shift + its documented base key |
| Backspace / Delete | Caps Shift + 0 |
| Arrow keys | Route to selected joystick mode; Cursor mode presses Caps+5/6/7/8 |
| Tab | Kempston fire |
| Option/Alt or Control/Ctrl alone | Symbol Shift |

Shifted punctuation chords own their Symbol modifier and suppress host Caps Shift. When a held chord includes another matrix key, its normal modifier remains active; repeated presses of a shared matrix position must preserve that key as held.

The event adapters are intentionally platform-specific: egui uses logical `Key` values, Windows uses Virtual-Key codes, GTK uses GDK keyvals (including shifted Unicode normalization), and macOS uses ANSI Carbon keycodes plus event flags. Keep those conversions at the host boundary. All hosts route arrows and Tab through the selected joystick mode; platform input adapters do not inject cursor keys directly. The Rust host keymap tests, shell keymap tests, and macOS host build are the contract checks; update them together when the mapping changes.
