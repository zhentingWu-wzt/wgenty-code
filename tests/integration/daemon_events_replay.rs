//! Acceptance: session event stream resume / replay (brief Step 1).
//!
//! Drives the real HTTP + SSE stack (`GET /sessions/:id/events`) with events
//! published through `DaemonEventSink` — the same dual-write (hub + replay
//! buffer) the server-side run loop uses — so replay, seam dedup, and
//! `sync_lost` are exercised end-to-end.

use crate::daemon_harness::{
    create_session, spawn_daemon, spawn_daemon_custom, SseReader, TestDaemon,
};
use wgenty_code::agent::runtime::{EventSink, RuntimeEvent};
use wgenty_code::daemon::run_loop::DaemonEventSink;

/// A sink that publishes `ContentDelta` events for `session_id` with the
/// daemon's own seq counter + replay buffer (the run loop's write path).
fn test_sink(d: &TestDaemon, session_id: &str, run_id: &str) -> DaemonEventSink {
    DaemonEventSink::new(
        session_id.to_string(),
        run_id.to_string(),
        d.state.session_event_hub.clone(),
        d.state.session_seq_counter(session_id),
        d.state.session_buffer(session_id),
    )
}

fn emit_deltas(sink: &DaemonEventSink, range: std::ops::RangeInclusive<u64>) {
    for i in range {
        sink.emit(RuntimeEvent::ContentDelta(format!("delta-{i}")));
    }
}

async fn read_seqs(reader: &mut SseReader, n: usize) -> Vec<u64> {
    let mut seqs = Vec::with_capacity(n);
    for _ in 0..n {
        let ev = reader.next_json().await;
        seqs.push(ev["seq"].as_u64().expect("event seq"));
    }
    seqs
}

/// Brief Step 1.2: subscriber A disconnects and reconnects with its last
/// seen seq as `after=`; it must receive exactly the missed events in order
/// and then continue live — matching subscriber B's sequence with no
/// duplicates and no gaps.
#[tokio::test]
async fn replay_after_reconnect_matches_live_subscriber_sequence() {
    let d = spawn_daemon().await;
    let sid = create_session(&d, "replay").await;
    let sink = test_sink(&d, &sid, "run-1");

    let events_url = || format!("{}/sessions/{sid}/events", d.base);
    let mut a = SseReader::connect(&d.client, &events_url()).await;
    let mut b = SseReader::connect(&d.client, &events_url()).await;

    // Both subscribers see the first three events live.
    emit_deltas(&sink, 1..=3);
    assert_eq!(read_seqs(&mut a, 3).await, vec![1, 2, 3]);
    assert_eq!(read_seqs(&mut b, 3).await, vec![1, 2, 3]);

    // A disconnects; events 4..=6 are only seen by B.
    drop(a);
    emit_deltas(&sink, 4..=6);
    assert_eq!(read_seqs(&mut b, 3).await, vec![4, 5, 6]);

    // A reconnects with after=3: replays 4..=6 in order, then attaches live.
    let mut a = SseReader::connect(&d.client, &format!("{}?after=3", events_url())).await;
    assert_eq!(read_seqs(&mut a, 3).await, vec![4, 5, 6], "replayed misses");

    // A live event reaches both exactly once (seam dedup on the rejoined A).
    emit_deltas(&sink, 7..=7);
    assert_eq!(read_seqs(&mut a, 1).await, vec![7]);
    assert_eq!(read_seqs(&mut b, 1).await, vec![7]);
}

/// Brief Step 1.3: when `after=` points at evicted seqs — or the buffer is
/// empty (post-restart) — the connection receives `sync_lost`; the recovery
/// convention (full `GET /sessions/:id`, then re-subscribe) works.
#[tokio::test]
async fn sync_lost_on_evicted_or_empty_buffer_then_full_recovery() {
    // Tiny replay buffer so eviction is cheap to trigger.
    let d = spawn_daemon_custom(|settings| settings.daemon.event_buffer_capacity = 4, |_| {}).await;
    let sid = create_session(&d, "evicted").await;
    let sink = test_sink(&d, &sid, "run-1");

    // 10 events with capacity 4 → seqs 1..=6 evicted.
    emit_deltas(&sink, 1..=10);

    // after=0: requested window no longer buffered → sync_lost (evicted),
    // and the stream stays open for live events past latest_seq.
    let url = format!("{}/sessions/{sid}/events?after=0", d.base);
    let mut conn = SseReader::connect(&d.client, &url).await;
    let lost = conn.next_json().await;
    assert_eq!(lost["kind"], "sync_lost");
    assert_eq!(lost["seq"], 0, "sync_lost is control-plane");
    assert_eq!(lost["data"]["reason"], "evicted");
    assert_eq!(lost["data"]["latest_seq"], 10);
    emit_deltas(&sink, 11..=11);
    let live = conn.next_json().await;
    assert_eq!(live["seq"], 11, "stream stays open after sync_lost");

    // Empty buffer (fresh session, simulating a daemon restart where the
    // in-memory buffer is gone): after= also yields sync_lost.
    let sid2 = create_session(&d, "restarted").await;
    let url2 = format!("{}/sessions/{sid2}/events?after=3", d.base);
    let mut conn2 = SseReader::connect(&d.client, &url2).await;
    let lost2 = conn2.next_json().await;
    assert_eq!(lost2["kind"], "sync_lost");
    assert_eq!(lost2["data"]["reason"], "evicted");
    assert_eq!(
        lost2["data"]["latest_seq"], 0,
        "nothing buffered post-restart"
    );

    // Recovery convention: full GET realigns on the persisted state…
    let full = d
        .client
        .get(format!("{}/sessions/{sid2}", d.base))
        .send()
        .await
        .expect("full GET");
    assert_eq!(full.status(), reqwest::StatusCode::OK);
    // …then a fresh subscription (no after) receives live events.
    let mut conn3 =
        SseReader::connect(&d.client, &format!("{}/sessions/{sid2}/events", d.base)).await;
    let sink2 = test_sink(&d, &sid2, "run-2");
    emit_deltas(&sink2, 1..=1);
    let ev = conn3.next_json().await;
    assert_eq!(ev["seq"], 1);
    assert_eq!(ev["kind"], "content_delta");
}

/// Brief Step 1.4: a subscriber that falls behind the broadcast window gets
/// an out-of-band `sync_lost` (reason=lagged) on ITS connection only; the
/// stream stays open and other subscribers are unaffected.
#[tokio::test(flavor = "current_thread")]
async fn lagged_subscriber_gets_sync_lost_others_unaffected() {
    // Hub capacity 2: publishing a burst without yielding guarantees the
    // SSE forwarder task's broadcast receiver lags (current-thread runtime,
    // no awaits while publishing — mirrors the run_loop unit test).
    let d = spawn_daemon_custom(
        |_| {},
        |state| {
            state.session_event_hub = tokio::sync::broadcast::channel(2).0;
        },
    )
    .await;
    let sid = create_session(&d, "lagged").await;
    let sink = test_sink(&d, &sid, "run-1");

    let events_url = || format!("{}/sessions/{sid}/events", d.base);
    let mut slow = SseReader::connect(&d.client, &events_url()).await;

    // Burst of 10 with no await in between: the forwarder cannot drain.
    emit_deltas(&sink, 1..=10);

    // This connection alone gets sync_lost(lagged)…
    let lost = slow.next_json().await;
    assert_eq!(lost["kind"], "sync_lost");
    assert_eq!(lost["data"]["reason"], "lagged");
    assert_eq!(lost["data"]["latest_seq"], 10);
    // …and the stream stays open for subsequent live events.
    emit_deltas(&sink, 11..=11);
    let live = slow.next_json().await;
    assert_eq!(live["seq"], 11);
    assert_eq!(live["kind"], "content_delta");

    // Another subscriber is unaffected: it receives live events and never a
    // sync_lost frame.
    let mut other = SseReader::connect(&d.client, &events_url()).await;
    emit_deltas(&sink, 12..=12);
    let ev = other.next_json().await;
    assert_eq!(ev["seq"], 12);
    assert_eq!(ev["kind"], "content_delta");

    // Sanity: the lagged connection's control frame never leaked into the
    // broadcast — the hub carries only real run events (kind != sync_lost).
    let mut raw = d.state.session_event_hub.subscribe();
    emit_deltas(&sink, 13..=13);
    let hub_ev = raw.recv().await.expect("hub event");
    assert_eq!(hub_ev.seq, 13);
}
