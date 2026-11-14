//! Dedicated Git preferences reader. Standard user/system config is trusted
//! configuration, not a document-directed path bridge. Repo includes stay denied.
use super::*;
const TOTAL: usize = 1024 * 1024;
struct Reader<'a> {
    root: &'a Path,
    signature: Vec<u8>,
    bytes: usize,
    files: usize,
    stack: HashSet<PathBuf>,
    autocrlf: String,
    safecrlf: String,
    ignorecase: bool,
    excludes: Option<bool>,
    mutation: Option<String>,
    head: Vec<u8>,
}
fn value(text: &str) -> Result<String, FileError> {
    let mut out = String::new();
    let mut quoted = false;
    let mut escape = false;
    for c in text.trim().chars() {
        if escape {
            out.push(match c {
                'n' => '\n',
                't' => '\t',
                'b' => '\u{8}',
                '"' => '"',
                '\\' => '\\',
                _ => {
                    return Err(unsupported(
                        "Unsupported Git config escape; use external Git.",
                    ));
                }
            });
            escape = false;
            continue;
        }
        if c == '\\' {
            escape = true;
        } else if c == '"' {
            quoted = !quoted;
        } else if !quoted && (c == '#' || c == ';') {
            break;
        } else {
            out.push(c);
        }
    }
    if quoted || escape {
        return Err(unsupported("Incomplete Git config value."));
    }
    Ok(out.trim().to_string())
}
fn boolean(v: &str) -> Option<bool> {
    match v.to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Some(true),
        "false" | "no" | "off" | "0" | "" => Some(false),
        _ => None,
    }
}
fn expand(path: &str, parent: &Path) -> Result<PathBuf, FileError> {
    if let Some(rest) = path.strip_prefix("~/") {
        Ok(PathBuf::from(std::env::var_os("HOME").ok_or_else(stale)?).join(rest))
    } else if path.starts_with('~') {
        Err(unsupported(
            "Named-user tilde config includes are not admitted.",
        ))
    } else if Path::new(path).is_absolute() {
        Ok(PathBuf::from(path))
    } else {
        Ok(parent.join(path))
    }
}
fn condition(condition: &str, root: &Path, parent: &Path, head: &[u8]) -> Result<bool, FileError> {
    let (kind, pattern) = condition.split_once(':').ok_or_else(|| {
        unsupported("Unknown conditional Git include; raw read-only comparison only.")
    })?;
    if pattern.len() > 4096 {
        return Err(error("capacity", "Git include condition exceeds 4 KiB."));
    }
    let (target, ci) = match kind {
        "gitdir" => (root.join(".git").to_string_lossy().into_owned(), false),
        "gitdir/i" => (root.join(".git").to_string_lossy().into_owned(), true),
        "worktree" => (root.to_string_lossy().into_owned(), false),
        "worktree/i" => (root.to_string_lossy().into_owned(), true),
        "onbranch" => (
            String::from_utf8_lossy(head)
                .trim()
                .strip_prefix("ref: refs/heads/")
                .unwrap_or("")
                .to_owned(),
            false,
        ),
        _ => {
            return Err(unsupported(
                "Unknown conditional Git include; raw read-only comparison only.",
            ));
        }
    };
    let mut pattern = pattern.to_string();
    if kind != "onbranch" {
        if pattern.starts_with("~/") {
            pattern = expand(&pattern, parent)?.to_string_lossy().into_owned();
        } else if let Some(rest) = pattern.strip_prefix("./") {
            pattern = parent.join(rest).to_string_lossy().into_owned();
        } else if !pattern.starts_with('/') {
            pattern = format!("**/{pattern}");
        }
    }
    if pattern.ends_with('/') {
        pattern.push_str("**");
    }
    let matcher = |p: &str| {
        globset::GlobBuilder::new(p)
            .literal_separator(true)
            .backslash_escape(true)
            .case_insensitive(ci)
            .build()
            .map(|g| g.compile_matcher())
            .map_err(|_| unsupported("Unsupported conditional Git glob."))
    };
    if matcher(&pattern)?.is_match(&target) {
        return Ok(true);
    }
    // Only gitdir also matches symlink spellings. worktree deliberately
    // matches the realpath location, without resolving an alias in the pattern.
    if matches!(kind, "gitdir" | "gitdir/i") {
        let prefix_end = pattern.find(['*', '?', '[']).unwrap_or(pattern.len());
        let literal = &pattern[..prefix_end];
        let prefix = if literal.ends_with('/') {
            Path::new(literal)
        } else {
            Path::new(literal).parent().unwrap_or(Path::new("/"))
        };
        if let Ok(real) = fs::canonicalize(prefix) {
            let suffix = &pattern[prefix.as_os_str().as_bytes().len()..];
            let p = format!("{}{suffix}", real.to_string_lossy());
            return Ok(matcher(&p)?.is_match(&target));
        }
    }
    Ok(false)
}
impl Reader<'_> {
    fn load(&mut self, path: &Path, local: bool, depth: usize) -> Result<(), FileError> {
        if depth > 8 || self.files >= 32 {
            return Err(error(
                "capacity",
                "Git config include budget reached (32 files / 8 levels).",
            ));
        }
        self.files += 1;
        self.signature
            .extend_from_slice(path.as_os_str().as_bytes());
        self.signature.push(0);
        let canonical = match fs::canonicalize(path) {
            Ok(p) => p,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !local => {
                self.signature.extend_from_slice(b"MISSING\0");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        if !self.stack.insert(canonical.clone()) {
            self.mutation =
                Some("Cyclic standard Git config include; raw read-only comparison only.".into());
            return Ok(());
        }
        let m = fs::metadata(&canonical)?;
        if !m.is_file() || (!local && m.uid() != 0 && m.uid() != unsafe { libc::geteuid() }) {
            return Err(unsupported("Git config ownership/type is unsupported."));
        }
        let bytes = private_store::read(&canonical, 256 * 1024)?;
        self.bytes += bytes.len();
        if self.bytes > TOTAL {
            return Err(error("capacity", "Git configuration exceeds 1 MiB."));
        }
        self.signature
            .extend_from_slice(canonical.as_os_str().as_bytes());
        self.signature.push(0);
        self.signature.extend_from_slice(&bytes);
        self.signature.push(0);
        let text =
            std::str::from_utf8(&bytes).map_err(|_| unsupported("Git config must be UTF-8."))?;
        let mut lines = vec![];
        let mut pending = String::new();
        for line in text.trim_start_matches('\u{feff}').lines() {
            pending.push_str(line);
            if pending.chars().rev().take_while(|c| *c == '\\').count() % 2 == 1 {
                pending.pop();
                continue;
            }
            lines.push(std::mem::take(&mut pending));
        }
        if !pending.is_empty() {
            return Err(unsupported("Incomplete Git config continuation."));
        }
        let mut section = String::new();
        let mut include = false;
        for line in lines {
            let l = line.trim();
            if l.is_empty() || l.starts_with(['#', ';']) {
                continue;
            }
            if l.starts_with('[') {
                let end = l.find(']').ok_or_else(stale)?;
                let header = l[1..end].trim();
                section = header
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .to_ascii_lowercase();
                // A subsection such as [core "other"] is not the global
                // core.autocrlf key. Never let it override effective transforms.
                if section != "includeif" && !header.eq_ignore_ascii_case(&section) {
                    section.clear();
                }
                include = false;
                if section == "include" || section == "includeif" {
                    if local {
                        return Err(unsupported("Repository config includes are not followed."));
                    }
                    if section == "include" {
                        include = true;
                    } else {
                        let cond = value(header.get(section.len()..).unwrap_or(""))?;
                        match condition(
                            &cond,
                            self.root,
                            path.parent().ok_or_else(stale)?,
                            &self.head,
                        ) {
                            Ok(found) => include = found,
                            Err(e) => {
                                self.mutation = Some(e.message);
                            }
                        }
                    }
                }
                continue;
            }
            let (key, raw) = l.split_once('=').unwrap_or((l, "true"));
            let key = key.trim().to_ascii_lowercase();
            if section == "include" || section == "includeif" {
                if include && key == "path" {
                    let path = expand(&value(raw)?, path.parent().ok_or_else(stale)?)?;
                    self.load(&path, false, depth + 1)?;
                }
                continue;
            }
            if section == "extensions" {
                return Err(unsupported(
                    "Repository/config extensions require external Git.",
                ));
            }
            if section != "core" {
                continue;
            }
            if ![
                "autocrlf",
                "safecrlf",
                "ignorecase",
                "attributesfile",
                "excludesfile",
                "worktree",
                "gitdir",
                "bare",
                "repositoryformatversion",
            ]
            .contains(&key.as_str())
            {
                continue;
            }
            let v = value(raw)?;
            match key.as_str() {
                "autocrlf" => {
                    self.autocrlf = if v.eq_ignore_ascii_case("input") {
                        "input".into()
                    } else {
                        boolean(&v)
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "unsupported".into())
                    }
                }
                "safecrlf" => {
                    self.safecrlf = boolean(&v)
                        .map(|v| v.to_string())
                        .unwrap_or(v.to_ascii_lowercase())
                }
                "ignorecase" => {
                    self.ignorecase =
                        boolean(&v).ok_or_else(|| unsupported("Invalid ignorecase boolean."))?
                }
                "attributesfile" => {
                    if !v.is_empty() {
                        self.mutation=Some("External attributes files are not followed; raw read-only comparison only.".into());
                    }
                }
                "excludesfile" => self.excludes = Some(!v.is_empty()),
                "worktree" | "gitdir" => {
                    return Err(unsupported(
                        "Redirected worktree/gitdir config is not admitted.",
                    ));
                }
                "bare" if boolean(&v) == Some(true) => {
                    return Err(unsupported("Bare repositories are not admitted."));
                }
                "repositoryformatversion" if v != "0" => {
                    return Err(unsupported(
                        "Only ordinary SHA-1 repository format 0 is admitted.",
                    ));
                }
                _ => {}
            }
        }
        self.stack.remove(&canonical);
        Ok(())
    }
}
pub(super) fn read(root: &Path, files: Vec<PathBuf>) -> Result<Policy, FileError> {
    for name in [
        "GIT_INDEX_FILE",
        "GIT_DIR",
        "GIT_COMMON_DIR",
        "GIT_WORK_TREE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_NOSYSTEM",
        "GIT_ATTR_NOSYSTEM",
    ] {
        if std::env::var_os(name).is_some() {
            return Err(unsupported("Git environment redirection is not admitted."));
        }
    }
    let head = private_store::read(&root.join(".git/HEAD"), 65536)?;
    let mut reader = Reader {
        root,
        signature: head.clone(),
        head,
        bytes: 0,
        files: 0,
        stack: HashSet::new(),
        autocrlf: "false".into(),
        safecrlf: "false".into(),
        ignorecase: false,
        excludes: None,
        mutation: None,
    };
    let mut default_ignore = false;
    for path in &files {
        if path.file_name().is_some_and(|n| n == "config")
            && let Some(parent) = path.parent()
        {
            let ignore = parent.join("ignore");
            let exists = fs::symlink_metadata(&ignore).is_ok();
            reader
                .signature
                .extend_from_slice(ignore.as_os_str().as_bytes());
            reader.signature.push(u8::from(exists));
            default_ignore |= exists;
        }
        reader.load(path, false, 0)?;
    }
    reader.load(&root.join(".git/config"), true, 0)?;
    if !["false", "true", "input"].contains(&reader.autocrlf.as_str()) {
        reader.mutation = Some("Unsupported autocrlf value; raw read-only comparison only.".into());
    }
    if reader.safecrlf != "false" && reader.autocrlf != "false" {
        reader.mutation = Some("safecrlf with conversion is read-only; use external Git.".into());
    }
    Ok(Policy {
        config: reader.signature,
        autocrlf: reader.autocrlf,
        ignorecase: reader.ignorecase,
        external_ignores: reader.excludes.unwrap_or(default_ignore),
        mutation: reader.mutation,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn standard_alias_include_order_and_conditions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            root.join(".git/config"),
            "[core]\nrepositoryformatversion=0\n",
        )
        .unwrap();
        fs::create_dir(dir.path().join("real")).unwrap();
        let target = dir.path().join("real/global.real");
        // Includes resolve relative to the original standard alias, not target.
        fs::write(dir.path().join("real/extra"), "[core]\nautocrlf=false\n").unwrap();
        fs::write(&target, "[include]\npath=extra\n").unwrap();
        fs::write(
            dir.path().join("extra"),
            format!(
                "[core]\nautocrlf=true\n[includeIf \"gitdir:{}\"]\npath=matching\n",
                root.join(".git").display()
            ),
        )
        .unwrap();
        fs::write(
            dir.path().join("matching"),
            "[core]\nautocrlf = input # comment\n[core \"other\"]\nautocrlf=false\n",
        )
        .unwrap();
        let canonical_root = fs::canonicalize(&root).unwrap();
        let linked_root = dir.path().join("repo-alias");
        std::os::unix::fs::symlink(&root, &linked_root).unwrap();
        assert!(
            !condition(
                &format!("worktree:{}", linked_root.display()),
                &canonical_root,
                dir.path(),
                b"ref: refs/heads/main\n"
            )
            .unwrap()
        );
        assert!(
            condition(
                &format!("gitdir:{}", linked_root.join(".git").display()),
                &canonical_root,
                dir.path(),
                b"ref: refs/heads/main\n"
            )
            .unwrap()
        );
        let alias = dir.path().join("global");
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        let p = read(&root, vec![alias.clone()]).unwrap();
        assert_eq!(p.autocrlf, "input");
        assert!(p.mutation.is_none());
        fs::write(root.join(".git/config"), "[core]\nautocrlf=false\n").unwrap();
        assert_eq!(read(&root, vec![alias]).unwrap().autocrlf, "false");
    }
}
