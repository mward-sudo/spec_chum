//! Trace unit tests.

use super::*;

#[test]
fn disabled_emit_is_noop() {
    let _g = test_lock();
    disable();
    clear();
    emit(EventKind::TapeRewind);
    assert_eq!(len(), 0);
}

#[test]
fn ring_keeps_last_n() {
    let _g = test_lock();
    disable();
    clear();
    enable(Category::TAPE);
    {
        let mut g = ring().lock().expect("lock");
        *g = Ring::new(4);
    }
    for i in 0..10u32 {
        emit(EventKind::TapePause { block: i });
    }
    let snap = snapshot();
    assert_eq!(snap.len(), 4);
    assert_eq!(snap[0].kind, EventKind::TapePause { block: 6 });
    assert_eq!(snap[3].kind, EventKind::TapePause { block: 9 });
    {
        let mut g = ring().lock().expect("lock");
        *g = Ring::new(DEFAULT_CAPACITY);
    }
    disable();
    clear();
}

#[test]
fn parse_categories() {
    assert!(Category::parse_list("tape,cpu").contains(Category::TAPE));
    assert!(Category::parse_list("tape,cpu").contains(Category::CPU));
    assert!(Category::parse_list("all").contains(Category::ULA));
    assert_eq!(
        Category::parse_list("default").bits(),
        Category::DEFAULT.bits()
    );
    assert_eq!(Category::default(), Category::NONE);
}

#[test]
fn dump_contains_flash_skip() {
    let _g = test_lock();
    disable();
    clear();
    enable(Category::TAPE);
    emit(EventKind::FlashLoadSkip {
        reason: FlashSkipReason::WrongFlag,
        block: 0,
        flag_got: 0xff,
        flag_want: 0x00,
        block_len: 19,
        want_len: 17,
    });
    let s = dump_string();
    assert!(s.contains("tape.flash.skip"));
    assert!(s.contains("wrong_flag"));
    disable();
    clear();
}

#[test]
fn append_flushes_events_to_trace_file() {
    let _g = test_lock();
    let path = std::env::temp_dir().join(format!(
        "spec_chum_append_{}_{}.txt",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    ));
    let _ = std::fs::remove_file(&path);
    configure_append_file_for_tests(&path);
    disable();
    clear();
    enable(Category::TAPE);
    emit(EventKind::TapeRewind);
    flush_append().expect("flush append");
    let body = std::fs::read_to_string(&path).unwrap_or_default();
    reset_append_sink_for_tests();
    let _ = std::fs::remove_file(&path);
    disable();
    clear();
    assert!(
        body.contains("tape.rewind"),
        "append file should contain the event, got {body:?}"
    );
}
