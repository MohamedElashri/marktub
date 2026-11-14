use super::*;
fn capture(text: &str) -> Capture {
    Capture::new(Path::new("/selected/review.md"), text.as_bytes().to_vec()).unwrap()
}
#[test]
fn baselines_freeze_and_only_displayed_saved_bytes_become_viewed() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = Reviews::new(dir.path().to_owned());
    assert!(!dir.path().join("review").exists());
    let v = r.begin(capture("# A\n\nOne.\n")).unwrap();
    assert!(v.initial);
    assert!(v.viewed_hash.is_none());
    r.displayed(&v.token, capture("# A\n\nOne.\n")).unwrap();
    let next = r.sync(&v.token, capture("# A\n\nTwo.\n")).unwrap();
    assert_eq!(next.reviewed_hash, v.reviewed_hash);
    assert!(next.viewed_hash.is_none());
    r.end(&v.token).unwrap();
    let again = r.begin(capture("# A\n\nTwo.\n")).unwrap();
    assert_eq!(again.viewed_hash, v.reviewed_hash);
    assert_eq!(again.reviewed_hash, v.reviewed_hash);
    let changed = r
        .update(&again.token, again.version, Action::Reviewed)
        .unwrap();
    assert_eq!(changed.reviewed_hash, Some(changed.current_hash.clone()));
    assert_eq!(changed.viewed_hash, v.reviewed_hash);
}
#[test]
fn verified_comments_are_durable_ambiguous_or_orphaned_and_exports_redact() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = Reviews::new(dir.path().to_owned());
    let v = r.begin(capture("# 文\r\n\r\nHello 🦀.\r\n")).unwrap();
    let start = v.text.find("Hello").unwrap();
    let quote = &v.text[start..];
    let v = r
        .update(
            &v.token,
            v.version,
            Action::Comment {
                start,
                end: v.text.len(),
                quote: quote.into(),
                text: "Please revise.".into(),
                heading_start: Some(0),
                heading_end: Some(7),
            },
        )
        .unwrap();
    let id = v.comments[0].comment.id.clone();
    assert_eq!(v.comments[0].status, "anchored");
    let v = r
        .sync(&v.token, capture("# 文\r\n\r\nHello 🦀.\r\nHello 🦀.\r\n"))
        .unwrap();
    assert_eq!(v.comments[0].status, "ambiguous");
    assert!(v.comments[0].start.is_none());
    let v = r.sync(&v.token, capture("# 文\n\nGone.\n")).unwrap();
    assert_eq!(v.comments[0].status, "orphaned");
    let report = r.export_view(&v.token, None, false).unwrap();
    assert!(!report.contains("Hello"));
    assert!(!report.contains("selected/review"));
    assert!(report.contains("orphaned"));
    let v = r
        .update(
            &v.token,
            v.version,
            Action::Reply {
                id: id.clone(),
                text: "Reply.".into(),
            },
        )
        .unwrap();
    assert_eq!(v.comments[0].comment.replies.len(), 1);
    assert!(
        r.update(&v.token, v.version - 1, Action::Delete { id })
            .is_err()
    );
    let record = r.session(&v.token).unwrap().record.clone();
    r.end(&v.token).unwrap();
    drop(r);
    let r = Reviews::new(dir.path().to_owned());
    let report = r.export_archive(&record, true).unwrap();
    assert!(report.contains("Hello"));
    assert!(report.contains("not_evaluated"));
}
#[test]
fn forged_ranges_storage_failure_unknown_schema_and_capacity_preserve_records() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = Reviews::new(dir.path().to_owned());
    let v = r.begin(capture("Start 🦀\n")).unwrap();
    assert!(
        r.update(
            &v.token,
            v.version,
            Action::Comment {
                start: 7,
                end: 8,
                quote: "x".into(),
                text: "Bad".into(),
                heading_start: None,
                heading_end: None
            }
        )
        .is_err()
    );
    crate::private_store::fail_next("");
    assert!(
        r.update(
            &v.token,
            v.version,
            Action::Decision {
                value: "approved".into()
            }
        )
        .is_err()
    );
    assert_eq!(r.view(&v.token).unwrap().decision, "pending");
    let id = r.session(&v.token).unwrap().record.clone();
    let path = r.record_path(&id).unwrap();
    let original = fs::read(&path).unwrap();
    fs::write(&path, b"{\"schema\":999}").unwrap();
    assert!(r.view(&v.token).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"{\"schema\":999}");
    fs::write(&path, original).unwrap();
    assert!(Capture::new(Path::new("/a"), vec![b'a'; CAPTURE_LIMIT as usize + 1]).is_err());
    r.delete(&id).unwrap();
    assert!(r.view(&v.token).is_err());
    assert!(r.list().unwrap().is_empty());
}

#[test]
fn unique_context_reanchors_manual_reanchor_and_decision_fences_are_exact() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = Reviews::new(dir.path().to_owned());
    let original = format!("{}\nQuote 🦀\n{}", "x".repeat(130), "y".repeat(130));
    let v = r.begin(capture(&original)).unwrap();
    let a = original.find("Quote").unwrap();
    let b = a + "Quote 🦀\n".len();
    let v = r
        .update(
            &v.token,
            v.version,
            Action::Comment {
                start: a,
                end: b,
                quote: original[a..b].into(),
                text: "Anchor".into(),
                heading_start: None,
                heading_end: None,
            },
        )
        .unwrap();
    let id = v.comments[0].comment.id.clone();
    let old = v.current_hash.clone();
    let changed = format!("Intro\n{original}");
    let v = r.sync(&v.token, capture(&changed)).unwrap();
    assert_eq!(v.comments[0].status, "reanchored");
    assert_eq!(v.comments[0].start, Some(a + 6));
    assert!(
        r.update_checked(&v.token, v.version, &old, Action::Reviewed)
            .is_err()
    );
    let v = r.sync(&v.token, capture("Replacement.\n")).unwrap();
    let v = r
        .update_checked(
            &v.token,
            v.version,
            &v.current_hash.clone(),
            Action::Reanchor {
                id: id.clone(),
                start: 0,
                end: 13,
                quote: "Replacement.\n".into(),
                heading_start: None,
                heading_end: None,
            },
        )
        .unwrap();
    assert_eq!(v.comments[0].status, "anchored");
    assert_eq!(v.comments[0].comment.previous.len(), 1);
    let v = r
        .update(&v.token, v.version, Action::Resolve { id, value: true })
        .unwrap();
    assert!(v.comments[0].comment.resolved);
    assert!(
        r.sync(
            &v.token,
            Capture::new(Path::new("/not-granted"), b"No".to_vec()).unwrap()
        )
        .is_err()
    );
}
#[test]
fn pinned_snapshots_hit_a_visible_cap_without_pruning_comments() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = Reviews::new(dir.path().to_owned());
    let mut v = r.begin(capture("Capture 0\n")).unwrap();
    for n in 0..8 {
        if n > 0 {
            v = r
                .sync(&v.token, capture(&format!("Capture {n}\n")))
                .unwrap();
        }
        v = r
            .update(
                &v.token,
                v.version,
                Action::Comment {
                    start: 0,
                    end: v.text.len(),
                    quote: v.text.clone(),
                    text: format!("Comment {n}"),
                    heading_start: None,
                    heading_end: None,
                },
            )
            .unwrap();
    }
    assert!(r.sync(&v.token, capture("Capture 9\n")).is_err());
    let after = r.view(&v.token).unwrap();
    assert_eq!(after.snapshots, 8);
    assert_eq!(after.comments.len(), 8);
    assert_eq!(after.current_hash, v.current_hash);
}

#[test]
fn stored_anchor_coordinates_are_reverified_and_crlf_end_lines_are_exact() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = Reviews::new(dir.path().to_owned());
    let v = r.begin(capture("A\r\n\r\n文 🦀\r\n")).unwrap();
    let start = v.text.find('文').unwrap();
    let v = r
        .update(
            &v.token,
            v.version,
            Action::Comment {
                start,
                end: v.text.len(),
                quote: v.text[start..].into(),
                text: "Check".into(),
                heading_start: None,
                heading_end: None,
            },
        )
        .unwrap();
    assert_eq!(v.comments[0].comment.anchor.line_start, 3);
    assert_eq!(v.comments[0].comment.anchor.line_end, 3);
    let id = r.session(&v.token).unwrap().record.clone();
    let mut data = r.load(&id).unwrap();
    data.comments[0].anchor.line_start = 99;
    private_store::atomic(
        &r.record_path(&id).unwrap(),
        &serde_json::to_vec(&data).unwrap(),
    )
    .unwrap();
    assert!(r.view(&v.token).is_err());
}

#[test]
fn reply_crud_and_decisions_preserve_anchor_and_zero_context_source_diff() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = Reviews::new(dir.path().to_owned());
    let v = r.begin(capture("Old.\n\nContext.\n")).unwrap();
    let v = r
        .update(
            &v.token,
            v.version,
            Action::Comment {
                start: 0,
                end: 5,
                quote: "Old.\n".into(),
                text: "Comment".into(),
                heading_start: None,
                heading_end: None,
            },
        )
        .unwrap();
    let id = v.comments[0].comment.id.clone();
    let original = v.comments[0].comment.anchor.quote_hash.clone();
    let v = r
        .update(
            &v.token,
            v.version,
            Action::Reply {
                id: id.clone(),
                text: "First".into(),
            },
        )
        .unwrap();
    let reply = v.comments[0].comment.replies[0].id.clone();
    let v = r
        .update(
            &v.token,
            v.version,
            Action::EditReply {
                id: id.clone(),
                reply: reply.clone(),
                text: "Edited".into(),
            },
        )
        .unwrap();
    assert_eq!(v.comments[0].comment.replies[0].text, "Edited");
    let v = r
        .update(
            &v.token,
            v.version,
            Action::DeleteReply {
                id: id.clone(),
                reply,
            },
        )
        .unwrap();
    assert!(v.comments[0].comment.replies.is_empty());
    assert_eq!(v.comments[0].comment.anchor.quote_hash, original);
    let v = r
        .update(
            &v.token,
            v.version,
            Action::Decision {
                value: "approved".into(),
            },
        )
        .unwrap();
    assert_eq!(v.decision_revision, Some(v.current_hash.clone()));
    let v = r.sync(&v.token, capture("New.\n\nContext.\n")).unwrap();
    assert_ne!(v.decision_revision, Some(v.current_hash.clone()));
    let diff = r.compare(&v.token, "reviewed").unwrap();
    assert_eq!(diff.hunks[0].old_lines, 1);
    assert_eq!(diff.hunks[0].new_lines, 1);
    assert_eq!(diff.hunks[0].old_start, 0);
    let v = r
        .update(&v.token, v.version, Action::Delete { id })
        .unwrap();
    assert!(v.comments.is_empty());
}

#[test]
fn disk_capture_alone_cannot_acknowledge_a_displayed_revision() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = Reviews::new(dir.path().to_owned());
    let original = capture("Displayed.\n");
    let v = r.begin(original.clone()).unwrap();
    let disk = capture("Disk only.\n");
    let v = r.sync(&v.token, disk.clone()).unwrap();
    assert!(
        r.displayed_revision(&v.token, &disk.key, disk.hash.clone())
            .is_err()
    );
    r.displayed_revision(&v.token, &original.key, original.hash.clone())
        .unwrap();
    let v = r.sync_grant(&v.token, disk.clone()).unwrap();
    r.displayed_revision(&v.token, &disk.key, disk.hash.clone())
        .unwrap();
    r.end(&v.token).unwrap();
    let next = r.begin(disk.clone()).unwrap();
    assert_eq!(next.viewed_hash, Some(disk.hash));
}

#[test]
fn multiple_records_survive_quota_interrupted_writes_and_failed_delete_catalogue() {
    let dir = tempfile::tempdir().unwrap();
    let mut reviews = Reviews::new(dir.path().to_owned());
    let first = reviews
        .begin(Capture::new(Path::new("/selected/first.txt"), b"First exact\n".to_vec()).unwrap())
        .unwrap();
    let second = reviews
        .begin(
            Capture::new(
                Path::new("/selected/second.txt"),
                b"Second exact\n".to_vec(),
            )
            .unwrap(),
        )
        .unwrap();
    let first_id = reviews.session(&first.token).unwrap().record.clone();
    let second_id = reviews.session(&second.token).unwrap().record.clone();
    let first_path = reviews.record_path(&first_id).unwrap();
    let second_path = reviews.record_path(&second_id).unwrap();
    let first_bytes = fs::read(&first_path).unwrap();
    let second_bytes = fs::read(&second_path).unwrap();
    let quota = dir.path().join("quota-fixture");
    fs::File::create(&quota)
        .unwrap()
        .set_len(private_store::STORE_QUOTA)
        .unwrap();
    assert!(
        reviews
            .update(
                &first.token,
                first.version,
                Action::Decision {
                    value: "approved".into()
                }
            )
            .err()
            .unwrap()
            .message
            .contains("budget")
    );
    assert_eq!(fs::read(&first_path).unwrap(), first_bytes);
    assert_eq!(fs::read(&second_path).unwrap(), second_bytes);
    fs::remove_file(quota).unwrap();
    private_store::fail_next("");
    assert!(
        reviews
            .update(&first.token, first.version, Action::Reviewed)
            .is_err()
    );
    fs::write(
        reviews.root.join(format!("{}.pending", Uuid::new_v4())),
        b"interrupted partial record",
    )
    .unwrap();
    drop(reviews);
    let mut reviews = Reviews::new(dir.path().to_owned());
    assert_eq!(reviews.list().unwrap().len(), 2);
    assert!(
        reviews
            .load(&first_id)
            .unwrap()
            .snapshots
            .values()
            .any(|text| text == "First exact\n")
    );
    reviews.export_archive(&first_id, true).unwrap();
    assert!(
        reviews
            .load(&second_id)
            .unwrap()
            .snapshots
            .values()
            .any(|text| text == "Second exact\n")
    );
    reviews.export_archive(&second_id, true).unwrap();
    private_store::fail_next("catalogue");
    assert!(reviews.delete(&first_id).is_err());
    assert_eq!(fs::read(&first_path).unwrap(), first_bytes);
    assert_eq!(fs::read(&second_path).unwrap(), second_bytes);
    assert_eq!(reviews.list().unwrap().len(), 2);
    let retained = first_path.with_extension("retained");
    fs::rename(&first_path, &retained).unwrap();
    fs::create_dir(&first_path).unwrap();
    assert!(reviews.delete(&first_id).is_err());
    assert_eq!(
        reviews.list().unwrap().len(),
        2,
        "unlink failure rolls back catalogue membership"
    );
    fs::remove_dir(&first_path).unwrap();
    fs::rename(&retained, &first_path).unwrap();
    assert_eq!(fs::read(&first_path).unwrap(), first_bytes);
    reviews.delete(&first_id).unwrap();
    assert_eq!(reviews.list().unwrap().len(), 1);
    assert_eq!(fs::read(&second_path).unwrap(), second_bytes);
}
