//! Sends through the real Quick Share client into the real Quick Share
//! server (both in one app instance) over TCP on localhost.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::*;
use crate::sender::SendItem;
use crate::transfers::{Direction, Status, Transfer};

fn wait_for(app: &App, what: &str, predicate: impl Fn(&Transfer) -> bool) -> Transfer {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(t) = app.transfers.lock().unwrap().iter().find(|t| predicate(t)) {
            return t.clone();
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

async fn wait(
    app: &Arc<App>,
    what: &'static str,
    predicate: impl Fn(&Transfer) -> bool + Send + 'static,
) -> Transfer {
    let app = app.clone();
    tokio::task::spawn_blocking(move || wait_for(&app, what, predicate))
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn phone_and_laptop_share_over_quick_share() {
    let home = tempfile::tempdir().unwrap();
    std::env::set_var("AIRBRIDGE_HOME", home.path());
    std::fs::create_dir_all(home.path().join("config")).unwrap();
    std::fs::write(
        home.path().join("config/settings.json"),
        r#"{"alias":"Test Laptop","notifications":false,"quickShare":false}"#,
    )
    .unwrap();
    let app = App::new().unwrap();
    let downloads: PathBuf = app.settings().download_dir.clone();

    // The receiving side: the real accept path, on localhost.
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(accept_loop(app.clone(), listener, CancellationToken::new()));
    let phone = crate::peers::Peer {
        info: DeviceInfo {
            alias: "Pixel".into(),
            version: "quickshare".into(),
            device_model: Some("Quick Share".into()),
            device_type: Some("mobile".into()),
            fingerprint: "quickshare:TEST".into(),
            port,
            protocol: "quickshare".into(),
            download: false,
        },
        ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
        kind: PeerKind::QuickShare,
        last_seen: std::time::Instant::now(),
        failures: 0,
        verified: false,
    };

    // 1. Files, including an empty one and one inside a folder; accepted.
    let source = tempfile::tempdir().unwrap();
    let photo: Vec<u8> = (0..1_700_000u32).map(|i| (i * 7 % 251) as u8).collect();
    std::fs::write(source.path().join("photo.jpg"), &photo).unwrap();
    std::fs::write(source.path().join("empty.txt"), b"").unwrap();
    std::fs::write(source.path().join("note.md"), b"# hello").unwrap();
    let items = vec![
        SendItem {
            path: source.path().join("photo.jpg"),
            name: "photo.jpg".into(),
            temporary: false,
        },
        SendItem {
            path: source.path().join("empty.txt"),
            name: "empty.txt".into(),
            temporary: false,
        },
        SendItem {
            path: source.path().join("note.md"),
            name: "Album/note.md".into(),
            temporary: false,
        },
    ];
    let out_id = send_files(&app, phone.clone(), items).unwrap();
    let incoming = wait(&app, "the incoming request", |t| {
        t.direction == Direction::In && t.status == Status::Waiting
    })
    .await;
    assert_eq!(incoming.peer["alias"], "Test Laptop");
    assert_eq!(incoming.files.len(), 3);
    let sent = wait(&app, "the PIN on the sender", {
        let out_id = out_id.clone();
        move |t| t.id == out_id && t.peer.get("pin").is_some()
    })
    .await;
    assert_eq!(
        sent.peer["pin"], incoming.peer["pin"],
        "both sides show the same PIN"
    );
    assert!(app.decide(&incoming.id, true));
    let received = wait(&app, "the received files", {
        let id = incoming.id.clone();
        move |t| t.id == id && t.status.is_final()
    })
    .await;
    assert_eq!(received.status, Status::Done, "{:?}", received.error);
    let sent = wait(&app, "the sender to finish", {
        let out_id = out_id.clone();
        move |t| t.id == out_id && t.status.is_final()
    })
    .await;
    assert_eq!(sent.status, Status::Done, "{:?}", sent.error);
    assert_eq!(std::fs::read(downloads.join("photo.jpg")).unwrap(), photo);
    assert_eq!(std::fs::read(downloads.join("empty.txt")).unwrap(), b"");
    assert_eq!(
        std::fs::read(downloads.join("Album/note.md")).unwrap(),
        b"# hello"
    );

    // 2. A link arrives as a message, without a prompt.
    let link_id = send_text(&app, phone.clone(), "https://example.com/recipe".into());
    let message = wait(&app, "the received link", |t| {
        t.direction == Direction::In && t.message.as_deref() == Some("https://example.com/recipe")
    })
    .await;
    assert_eq!(message.status, Status::Done);
    let sent = wait(&app, "the link sender", move |t| {
        t.id == link_id && t.status.is_final()
    })
    .await;
    assert_eq!(sent.status, Status::Done, "{:?}", sent.error);

    // 3. Declining is reported to the sender.
    let declined_id = send_files(
        &app,
        phone.clone(),
        vec![SendItem {
            path: source.path().join("note.md"),
            name: "declined.md".into(),
            temporary: false,
        }],
    )
    .unwrap();
    let asked = wait(&app, "the second request", |t| {
        t.direction == Direction::In && t.status == Status::Waiting
    })
    .await;
    assert!(app.decide(&asked.id, false));
    let sent = wait(&app, "the declined sender", move |t| {
        t.id == declined_id && t.status.is_final()
    })
    .await;
    assert_eq!(sent.status, Status::Declined);
    assert!(!downloads.join("declined.md").exists());

    // 4. The sender gives up while the receiver is still deciding.
    let cancelled_id = send_files(
        &app,
        phone,
        vec![SendItem {
            path: source.path().join("photo.jpg"),
            name: "never.jpg".into(),
            temporary: false,
        }],
    )
    .unwrap();
    let asked = wait(&app, "the third request", |t| {
        t.direction == Direction::In && t.status == Status::Waiting
    })
    .await;
    app.cancel(&cancelled_id);
    let gone = wait(&app, "the receiver to notice", move |t| {
        t.id == asked.id && t.status.is_final()
    })
    .await;
    assert_eq!(gone.status, Status::Cancelled);
    assert!(!downloads.join("never.jpg").exists());
}
