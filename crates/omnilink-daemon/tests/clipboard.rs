//! Integration tests for Wayland clipboard synchronization and BLAKE3 loop prevention.

use omnilink_daemon::clipboard::{
    is_sensitive_mime_type, normalize_mime_type, select_best_mime_type, spawn_clipboard_listener,
    ClipboardError, ClipboardEvent, ClipboardFilter, ClipboardListener, DEFAULT_FILTER_CAPACITY,
    SUPPORTED_MIME_TYPES,
};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

static TEST_MUTEX: Mutex<()> = Mutex::new(());

#[test]
fn test_clipboard_filter_lifecycle_and_eviction() {
    let mut filter = ClipboardFilter::new();
    assert_eq!(filter.capacity(), DEFAULT_FILTER_CAPACITY);
    assert_eq!(filter.len(), 0);

    // Fill ring buffer with exactly 16 distinct text snippets
    for i in 0..16 {
        let snippet = format!("https://github.com/omnilink/commit/{i:04x}");
        assert!(filter.should_sync(snippet.as_bytes()));
    }
    assert_eq!(filter.len(), 16);

    // Duplicate check on entry 0 and entry 15
    assert!(!filter.should_sync(b"https://github.com/omnilink/commit/0000"));
    assert!(!filter.should_sync(b"https://github.com/omnilink/commit/000f"));

    // Push 17th item
    let item_17 = b"https://github.com/omnilink/commit/0010";
    assert!(filter.should_sync(item_17));
    assert_eq!(filter.len(), 16);

    // Oldest item (0000) was evicted, so now should_sync returns true
    assert!(filter.should_sync(b"https://github.com/omnilink/commit/0000"));
    // Newest item (0010) is still in history
    assert!(!filter.should_sync(item_17));
}

#[test]
fn test_clipboard_filter_record_incoming_prevents_echo() {
    let mut filter = ClipboardFilter::new();

    // Injected Android clipboard payload
    let android_clip = b"SMS Verification Code: 839201";

    // Record foreign injection into filter
    filter.record(android_clip);
    assert_eq!(filter.len(), 1);
    assert!(filter.contains(android_clip));

    // When Wayland selection fires for the newly set desktop clip, transmission back to Android must be skipped
    assert!(!filter.should_sync(android_clip));
    assert!(filter.contains(android_clip));
}

#[test]
fn test_clipboard_event_mime_type_negotiation() {
    assert_eq!(
        SUPPORTED_MIME_TYPES,
        &["text/plain;charset=utf-8", "text/plain", "image/png"]
    );
    assert_eq!(
        normalize_mime_type("text/plain; charset=utf-8"),
        "text/plain;charset=utf-8"
    );

    // Prioritize UTF-8 plain text over generic plain text
    let list1 = vec![
        "text/plain".into(),
        "text/plain;charset=utf-8".into(),
        "text/html".into(),
    ];
    assert_eq!(select_best_mime_type(&list1), Some("text/plain;charset=utf-8"));

    // Fall back to text/plain if charset not specified
    let list2 = vec!["text/plain".into(), "text/html".into()];
    assert_eq!(select_best_mime_type(&list2), Some("text/plain"));

    // Image PNG selection
    let list3 = vec!["image/png".into(), "image/jpeg".into()];
    assert_eq!(select_best_mime_type(&list3), Some("image/png"));

    // Unsupported returns None
    let list4 = vec!["application/x-tar".into()];
    assert_eq!(select_best_mime_type(&list4), None);

    let event = ClipboardEvent::new("image/png", false, vec![1, 2, 3]);
    assert_eq!(event.mime_type, "image/png");
}

#[test]
fn test_sensitive_clipboard_markers() {
    assert!(is_sensitive_mime_type("x-kde-passwordManagerHint"));
    assert!(is_sensitive_mime_type("application/x-kde-passwordManagerHint"));
    assert!(is_sensitive_mime_type("application/x-secret-service"));
    assert!(is_sensitive_mime_type("application/x-keepassx-password"));
    assert!(is_sensitive_mime_type("application/x-1password"));

    assert!(!is_sensitive_mime_type("text/plain"));
    assert!(!is_sensitive_mime_type("image/png"));
}

#[test]
fn test_wayland_display_missing_graceful_error() {
    let _lock = TEST_MUTEX.lock().unwrap();

    let saved_display = std::env::var_os("WAYLAND_DISPLAY");
    let saved_socket = std::env::var_os("WAYLAND_SOCKET");

    std::env::remove_var("WAYLAND_DISPLAY");
    std::env::remove_var("WAYLAND_SOCKET");

    assert!(!ClipboardListener::is_wayland_available());

    let (tx, _rx) = mpsc::channel(8);
    let filter = Arc::new(Mutex::new(ClipboardFilter::new()));

    let result = spawn_clipboard_listener(tx.clone(), filter);
    match result {
        Err(ClipboardError::NoWaylandDisplay) => {}
        other => panic!("Expected Err(ClipboardError::NoWaylandDisplay), got: {other:?}"),
    }

    let listener = ClipboardListener::new(tx);
    let shutdown = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let run_res = listener.run(shutdown);
    match run_res {
        Err(ClipboardError::NoWaylandDisplay) => {}
        other => panic!("Expected Err(ClipboardError::NoWaylandDisplay), got: {other:?}"),
    }

    if let Some(d) = saved_display {
        std::env::set_var("WAYLAND_DISPLAY", d);
    }
    if let Some(s) = saved_socket {
        std::env::set_var("WAYLAND_SOCKET", s);
    }
}
