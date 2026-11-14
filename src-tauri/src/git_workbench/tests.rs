use super::*;
use std::process::Command;
fn git(root: &Path, args: &[&str]) -> String {
    let result = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "protocol.allow=never",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ALLOW_PROTOCOL", "")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}
struct Fixture {
    root: tempfile::TempDir,
    work: Workbench,
    base: String,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        git(root.path(), &["init", "-b", "main"]);
        git(root.path(), &["config", "user.name", "Test"]);
        git(
            root.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        git(root.path(), &["config", "core.autocrlf", "false"]);
        let base = (0..40)
            .map(|i| format!("line {i:02}\n"))
            .collect::<String>();
        fs::write(root.path().join("source.txt"), &base).unwrap();
        fs::write(root.path().join("other.txt"), "other base\n").unwrap();
        git(root.path(), &["add", "--", "source.txt", "other.txt"]);
        git(root.path(), &["commit", "-m", "base"]);
        let store = root.path().join("private");
        private_store::directory(&store).unwrap();
        Self {
            root,
            work: Workbench::new(store.clone(), vec![store]),
            base,
        }
    }
    fn changed(&mut self) -> State {
        fs::write(
            self.root.path().join("source.txt"),
            self.base
                .replace("line 01", "first 🦀")
                .replace("line 35", "second 文"),
        )
        .unwrap();
        self.work.choose(self.root.path()).unwrap()
    }
    fn source(&self, s: &State) -> String {
        s.files
            .iter()
            .find(|f| f.path == "source.txt")
            .unwrap()
            .id
            .clone()
    }
}
#[test]
fn selected_hunk_matches_git_and_preserves_other_entries() {
    let mut f = Fixture::new();
    let s = f.changed();
    let before = f.work.grant(&s.id).unwrap().index().unwrap();
    let d = f
        .work
        .compare(&s.id, &f.source(&s), "index", "worktree")
        .unwrap();
    assert_eq!(d.hunks.len(), 2);
    f.work
        .mutate(&s.id, &d.id, "stage", &[d.hunks[0].id.clone()])
        .unwrap();
    assert_eq!(
        git(f.root.path(), &["show", ":source.txt"]),
        f.base.replace("line 01", "first 🦀")
    );
    assert_eq!(
        fs::read_to_string(f.root.path().join("source.txt")).unwrap(),
        f.base
            .replace("line 01", "first 🦀")
            .replace("line 35", "second 文")
    );
    let after = f.work.grant(&s.id).unwrap().index().unwrap();
    assert_eq!(
        before.get(b"other.txt").unwrap().raw,
        after.get(b"other.txt").unwrap().raw
    );
    git(f.root.path(), &["write-tree"]);
    let s = f.work.refresh(&s.id, 0).unwrap();
    let d = f
        .work
        .compare(&s.id, &f.source(&s), "head", "index")
        .unwrap();
    f.work
        .mutate(
            &s.id,
            &d.id,
            "unstage",
            &d.hunks.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
        )
        .unwrap();
    assert_eq!(git(f.root.path(), &["show", ":source.txt"]), f.base);
}
#[test]
fn lock_contention_and_noncooperating_index_change_refuse() {
    let mut f = Fixture::new();
    let s = f.changed();
    let d = f
        .work
        .compare(&s.id, &f.source(&s), "index", "worktree")
        .unwrap();
    let index = f.root.path().join(".git/index");
    let original = fs::read(&index).unwrap();
    let lock = f.root.path().join(".git/index.lock");
    fs::write(&lock, b"foreign").unwrap();
    assert_eq!(
        f.work
            .mutate(&s.id, &d.id, "stage", &[d.hunks[0].id.clone()])
            .err()
            .unwrap()
            .code,
        "git-lock"
    );
    assert_eq!(fs::read(&lock).unwrap(), b"foreign");
    fs::remove_file(&lock).unwrap();
    let path = index.clone();
    let altered = Index::parse(original.clone())
        .unwrap()
        .updated(
            b"other.txt",
            Some((oid(b"other changed\n").unwrap(), 14, 0o100644)),
        )
        .unwrap();
    let expected = altered.clone();
    assert_eq!(
        f.work
            .mutate_hook(&s.id, &d.id, "stage", &[d.hunks[0].id.clone()], move || {
                fs::write(path, altered).unwrap()
            })
            .err()
            .unwrap()
            .code,
        "git-stale"
    );
    assert_eq!(fs::read(index).unwrap(), expected);
    assert!(!lock.exists());
}
#[test]
fn replacing_our_lock_never_removes_a_foreign_lock() {
    let mut f = Fixture::new();
    let s = f.changed();
    let d = f
        .work
        .compare(&s.id, &f.source(&s), "index", "worktree")
        .unwrap();
    let lock = f.root.path().join(".git/index.lock");
    let copy = lock.clone();
    assert!(
        f.work
            .mutate_hook(&s.id, &d.id, "stage", &[d.hunks[0].id.clone()], move || {
                fs::remove_file(&copy).unwrap();
                fs::write(copy, b"replacement owner").unwrap();
            })
            .is_err()
    );
    assert_eq!(fs::read(lock).unwrap(), b"replacement owner");
}
#[test]
fn stale_worktree_and_head_or_wrong_hunks_refuse() {
    let mut f = Fixture::new();
    let s = f.changed();
    let d = f
        .work
        .compare(&s.id, &f.source(&s), "index", "worktree")
        .unwrap();
    assert!(
        f.work
            .mutate(&s.id, &d.id, "stage", &["forged".into()])
            .is_err()
    );
    fs::write(f.root.path().join("source.txt"), "new concurrent bytes\n").unwrap();
    assert_eq!(
        f.work
            .mutate(&s.id, &d.id, "stage", &[d.hunks[0].id.clone()])
            .err()
            .unwrap()
            .code,
        "git-stale"
    );
    let s = f.work.refresh(&s.id, 0).unwrap();
    let d = f
        .work
        .compare(&s.id, &f.source(&s), "index", "worktree")
        .unwrap();
    git(
        f.root.path(),
        &["commit", "--allow-empty", "-m", "new head"],
    );
    assert_eq!(
        f.work
            .mutate(&s.id, &d.id, "stage", &[d.hunks[0].id.clone()])
            .err()
            .unwrap()
            .code,
        "git-stale"
    );
}
#[test]
fn unknown_extensions_split_and_checksum_fail_closed() {
    let mut f = Fixture::new();
    let s = f.changed();
    let path = f.root.path().join(".git/index");
    let original = fs::read(&path).unwrap();
    for name in [b"TEST", b"link", b"sdir", b"FSMN"] {
        let mut b = original[..original.len() - 20].to_vec();
        b.extend(name);
        b.extend(4u32.to_be_bytes());
        b.extend(b"data");
        let h = Sha1::digest(&b);
        b.extend(h);
        fs::write(&path, b).unwrap();
        let s = f.work.refresh(&s.id, 0).unwrap();
        let d = f
            .work
            .compare(&s.id, &f.source(&s), "index", "worktree")
            .unwrap();
        assert!(d.mutation.is_some());
        assert!(
            f.work
                .mutate(&s.id, &d.id, "stage", &[d.hunks[0].id.clone()])
                .is_err()
        );
    }
    let mut corrupt = original;
    corrupt[20] ^= 1;
    assert!(Index::parse(corrupt).is_err());
}
#[test]
fn attributes_and_configured_drivers_never_execute() {
    let mut f = Fixture::new();
    let s = f.changed();
    let marker = f.root.path().join("EXECUTED");
    let mut config = fs::read_to_string(f.root.path().join(".git/config")).unwrap();
    config.push_str(&format!("\n[filter \"evil\"]\n clean = touch {}\n smudge = touch {}\n[diff \"evil\"]\n textconv = touch {}\n",marker.display(),marker.display(),marker.display()));
    fs::write(f.root.path().join(".git/config"), config).unwrap();
    fs::write(
        f.root.path().join(".gitattributes"),
        "*.txt filter=evil diff=evil working-tree-encoding=UTF-16\n",
    )
    .unwrap();
    let s = f.work.refresh(&s.id, 0).unwrap();
    let d = f
        .work
        .compare(&s.id, &f.source(&s), "index", "worktree")
        .unwrap();
    assert!(d.mutation.is_some());
    assert!(
        f.work
            .mutate(&s.id, &d.id, "stage", &[d.hunks[0].id.clone()])
            .is_err()
    );
    assert!(!marker.exists());
}
#[test]
fn crlf_clean_and_discard_recovery_are_exact() {
    let mut f = Fixture::new();
    git(f.root.path(), &["config", "core.autocrlf", "true"]);
    let bytes = f.base.replace("line 01", "first 🦀").replace('\n', "\r\n");
    fs::write(f.root.path().join("source.txt"), &bytes).unwrap();
    let s = f.work.choose(f.root.path()).unwrap();
    let d = f
        .work
        .compare(&s.id, &f.source(&s), "index", "worktree")
        .unwrap();
    f.work
        .mutate(&s.id, &d.id, "stage", &[d.hunks[0].id.clone()])
        .unwrap();
    assert_eq!(
        git(f.root.path(), &["show", ":source.txt"]),
        f.base.replace("line 01", "first 🦀")
    );
    assert_eq!(
        fs::read_to_string(f.root.path().join("source.txt")).unwrap(),
        bytes
    );
    fs::write(
        f.root.path().join("source.txt"),
        bytes.replace("line 35", "second 文"),
    )
    .unwrap();
    let s = f.work.refresh(&s.id, 0).unwrap();
    let d = f
        .work
        .compare(&s.id, &f.source(&s), "index", "worktree")
        .unwrap();
    let result = f
        .work
        .mutate(
            &s.id,
            &d.id,
            "discard",
            &d.hunks.iter().map(|h| h.id.clone()).collect::<Vec<_>>(),
        )
        .unwrap();
    assert_eq!(
        fs::read_to_string(f.root.path().join("source.txt")).unwrap(),
        bytes
    );
    assert_eq!(
        f.work.backup_bytes(&s.id, &result.backup.unwrap()).unwrap(),
        bytes.replace("line 35", "second 文").as_bytes()
    );
}
#[test]
fn unusual_paths_new_empty_file_binary_symlink_and_revoke() {
    let mut f = Fixture::new();
    fs::write(f.root.path().join("文 space.txt"), b"").unwrap();
    let s = f.work.choose(f.root.path()).unwrap();
    let file = s
        .files
        .iter()
        .find(|x| x.path == "文 space.txt")
        .unwrap()
        .id
        .clone();
    let d = f.work.compare(&s.id, &file, "index", "worktree").unwrap();
    f.work
        .mutate(&s.id, &d.id, "stage", &[d.hunks[0].id.clone()])
        .unwrap();
    assert!(git(f.root.path(), &["ls-files", "--", "文 space.txt"]).contains("space.txt"));
    fs::write(f.root.path().join("source.txt"), b"binary\0\xff").unwrap();
    let s = f.work.refresh(&s.id, 0).unwrap();
    assert!(
        f.work
            .compare(&s.id, &f.source(&s), "index", "worktree")
            .is_err()
    );
    std::os::unix::fs::symlink("/etc/passwd", f.root.path().join("escape")).unwrap();
    let s = f.work.refresh(&s.id, 0).unwrap();
    let file = s.files.iter().find(|x| x.path == "escape").unwrap();
    assert!(f.work.scoped(&s.id, &file.id).is_err());
    f.work.revoke(&s.id).unwrap();
    assert!(f.work.refresh(&s.id, 0).is_err());
}
#[test]
fn large_diff_pages_are_bounded_and_history_is_native() {
    let mut f = Fixture::new();
    let base = (0..10_000)
        .map(|i| format!("const line{i} = {i};\n"))
        .collect::<String>();
    fs::write(f.root.path().join("source.txt"), &base).unwrap();
    git(f.root.path(), &["add", "source.txt"]);
    git(f.root.path(), &["commit", "-m", "ten thousand lines"]);
    fs::write(
        f.root.path().join("source.txt"),
        base.replace(" = ", " = changed + "),
    )
    .unwrap();
    let s = f.work.choose(f.root.path()).unwrap();
    assert_eq!(s.history.len(), 2);
    let d = f
        .work
        .compare(&s.id, &f.source(&s), "index", "worktree")
        .unwrap();
    assert_eq!(d.unified_rows, 20_000);
    assert_eq!(d.split_rows, 10_000);
    assert_eq!(
        f.work
            .page(&s.id, &d.id, 19_900, false, false)
            .unwrap()
            .len(),
        100
    );
    let _ = f
        .work
        .compare(&s.id, &f.source(&s), &s.history[1].id, "worktree")
        .unwrap();
    assert!(f.work.page(&s.id, &d.id, 0, false, false).is_err());
}
#[test]
fn v4_and_conflicts_remain_read_only_with_head_disk_inspection() {
    let mut f = Fixture::new();
    git(f.root.path(), &["update-index", "--index-version=4"]);
    let s = f.changed();
    let d = f
        .work
        .compare(&s.id, &f.source(&s), "index", "worktree")
        .unwrap();
    assert!(d.mutation.is_some());
    assert!(
        f.work
            .mutate(&s.id, &d.id, "stage", &[d.hunks[0].id.clone()])
            .is_err()
    );
    git(f.root.path(), &["update-index", "--index-version=2"]);
    git(f.root.path(), &["checkout", "--", "source.txt"]);
    git(f.root.path(), &["checkout", "-b", "side"]);
    fs::write(f.root.path().join("source.txt"), "side\n").unwrap();
    git(f.root.path(), &["commit", "-am", "side"]);
    git(f.root.path(), &["checkout", "main"]);
    fs::write(f.root.path().join("source.txt"), "main\n").unwrap();
    git(f.root.path(), &["commit", "-am", "main"]);
    let _ = Command::new("git")
        .arg("-C")
        .arg(f.root.path())
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "merge",
            "--no-edit",
            "side",
        ])
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    let s = f.work.choose(f.root.path()).unwrap();
    assert!(
        f.work
            .compare(&s.id, &f.source(&s), "index", "worktree")
            .is_err()
    );
    let d = f
        .work
        .compare(&s.id, &f.source(&s), "head", "worktree")
        .unwrap();
    assert!(d.mutation.is_some());
}
#[test]
fn history_includes_clean_files_and_nested_ignores_are_honored() {
    let mut f = Fixture::new();
    fs::create_dir(f.root.path().join("nested")).unwrap();
    fs::write(f.root.path().join("nested/.gitignore"), "ignored.txt\n").unwrap();
    fs::write(f.root.path().join("nested/ignored.txt"), "secret\n").unwrap();
    fs::write(f.root.path().join("nested/visible.txt"), "visible\n").unwrap();
    let s = f.work.choose(f.root.path()).unwrap();
    assert!(!s.files.iter().any(|r| r.path == "nested/ignored.txt"));
    assert!(s.files.iter().any(|r| r.path == "nested/visible.txt"));
    let h = f.work.history_files(&s.id, &s.history[0].id, 0).unwrap();
    let file = h.files.iter().find(|r| r.path == "source.txt").unwrap();
    let d = f
        .work
        .compare(&s.id, &file.id, &s.history[0].id, "worktree")
        .unwrap();
    assert!(d.hunks.is_empty());
    assert!(d.mutation.is_some());
}
#[test]
fn bom_config_include_backup_failure_and_display_provenance() {
    let mut f = Fixture::new();
    let s = f.changed();
    let d = f
        .work
        .compare(&s.id, &f.source(&s), "index", "worktree")
        .unwrap();
    crate::private_store::fail_next("");
    assert!(
        f.work
            .mutate(
                &s.id,
                &d.id,
                "discard",
                &d.hunks.iter().map(|h| h.id.clone()).collect::<Vec<_>>()
            )
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(f.root.path().join("source.txt")).unwrap(),
        f.base
            .replace("line 01", "first 🦀")
            .replace("line 35", "second 文")
    );
    assert!(!f.root.path().join(".git/index.lock").exists());
    assert_eq!(source_lines(b"a\rb\r\nc\nd"), vec![1, 3, 4]);
    assert_eq!(display_line("\u{feff}a\rb\r\n"), "⟪BOM⟫a␍b");
    let config = fs::read_to_string(f.root.path().join(".git/config")).unwrap();
    fs::write(
        f.root.path().join(".git/config"),
        format!("\u{feff}{config}\n[include]\npath=/etc/passwd\n"),
    )
    .unwrap();
    assert!(f.work.refresh(&s.id, 0).is_err());
}

#[test]
fn badge_cache_is_scoped_survives_history_and_clears_on_revoke() {
    let mut f = Fixture::new();
    let s = f.changed();
    let root = fs::canonicalize(f.root.path()).unwrap();
    let paths = vec![
        ("source-handle".into(), root.join("source.txt")),
        ("clean-handle".into(), root.join("other.txt")),
        (
            "outside-handle".into(),
            PathBuf::from("/outside/source.txt"),
        ),
    ];
    let badges = f.work.badges(paths.clone());
    assert_eq!(badges.len(), 1);
    assert!(badges.contains_key("source-handle"));
    let commit = s.history[0].id.clone();
    f.work.history_files(&s.id, &commit, 0).unwrap();
    assert_eq!(f.work.badges(paths.clone()), badges);
    f.work.revoke(&s.id).unwrap();
    assert!(f.work.badges(paths).is_empty());
}

#[test]
fn compressed_blob_index_capacity_and_repeated_foreign_lock_are_refused_without_mutation() {
    let mut f = Fixture::new();
    let s = f.changed();
    let d = f
        .work
        .compare(&s.id, &f.source(&s), "index", "worktree")
        .unwrap();
    let lock = f.root.path().join(".git/index.lock");
    fs::write(&lock, b"foreign stress owner").unwrap();
    let index = f.root.path().join(".git/index");
    let before = fs::read(&index).unwrap();
    for _ in 0..32 {
        assert!(
            f.work
                .mutate(&s.id, &d.id, "stage", &[d.hunks[0].id.clone()])
                .is_err()
        );
        assert_eq!(fs::read(&lock).unwrap(), b"foreign stress owner");
        assert_eq!(fs::read(&index).unwrap(), before);
    }
    fs::remove_file(&lock).unwrap();
    let oid = git2::Repository::open(f.root.path())
        .unwrap()
        .blob(&vec![b'x'; FILE_LIMIT + 1])
        .unwrap();
    git(
        f.root.path(),
        &[
            "update-index",
            "--cacheinfo",
            &format!("100644,{oid},source.txt"),
        ],
    );
    let oversized_index = fs::read(&index).unwrap();
    let state = f.work.refresh(&s.id, 0).unwrap();
    assert!(
        f.work
            .compare(&state.id, &f.source(&state), "index", "worktree")
            .is_err()
    );
    assert_eq!(fs::read(&index).unwrap(), oversized_index);
    let file = fs::OpenOptions::new().write(true).open(&index).unwrap();
    file.set_len(INDEX_LIMIT + 1).unwrap();
    let bad_index = fs::read(&index).unwrap();
    assert!(
        f.work
            .compare(&state.id, &f.source(&state), "index", "worktree")
            .is_err()
    );
    assert_eq!(fs::read(&index).unwrap(), bad_index);
}
