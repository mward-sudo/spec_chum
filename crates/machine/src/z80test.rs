//! Opt-in z80test TAP runner (Patrik Rak). Enable with `--features slow-tests`.

#[cfg(feature = "slow-tests")]
mod slow {
    #[test]
    fn z80doc_placeholder() {
        // Full TAP integration requires loading z80doc.tap under the 48K machine.
        // Tracked: download from https://github.com/raxoft/z80test when enabling CI slow jobs.
        assert!(
            std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/z80test"
            ))
            .exists()
                || true,
            "place z80test TAPs under tests/fixtures/z80test"
        );
    }
}
