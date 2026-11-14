//! Native OS/CLI input only. No renderer-supplied path can enter this queue.
use std::{
    collections::VecDeque,
    path::PathBuf,
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct OpenRequests {
    paths: VecDeque<(PathBuf, Instant)>,
    rejected: bool,
}
impl OpenRequests {
    pub fn push(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for (index, path) in paths.into_iter().enumerate() {
            if index >= 16 {
                self.rejected = true;
                break;
            }
            if self.paths.len() >= 16 {
                self.rejected = true;
                break;
            }
            if path.as_os_str().len() > 4096
                || !matches!(
                    path.extension()
                        .and_then(|s| s.to_str())
                        .map(str::to_ascii_lowercase)
                        .as_deref(),
                    Some("md" | "markdown" | "txt")
                )
            {
                self.rejected = true;
                continue;
            }
            if !self.paths.iter().any(|(p, _)| p == &path) {
                self.paths.push_back((path, Instant::now()));
            }
        }
    }
    pub fn arguments(&mut self, args: impl IntoIterator<Item = String>, cwd: &std::path::Path) {
        let mut literal = false;
        let mut paths = Vec::new();
        for (index, arg) in args.into_iter().skip(1).enumerate() {
            if index >= 17 {
                self.rejected = true;
                break;
            }
            if arg == "--" && !literal {
                literal = true;
                continue;
            }
            // The upstream handoff converts invalid Unicode lossily; refuse replacement chars.
            if (!literal && arg.starts_with('-')) || arg.contains('\u{fffd}') || arg.contains("://")
            {
                self.rejected = true;
                continue;
            }
            paths.push(cwd.join(arg));
        }
        self.push(paths);
    }
    pub fn take(&mut self) -> (Vec<PathBuf>, bool) {
        let mut expired = false;
        let paths = self
            .paths
            .drain(..)
            .filter_map(|(p, at)| {
                if at.elapsed() < Duration::from_secs(120) {
                    Some(p)
                } else {
                    expired = true;
                    None
                }
            })
            .collect();
        (paths, std::mem::take(&mut self.rejected) || expired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_arguments_are_bounded_and_never_flags_urls_or_lossy_paths() {
        let mut q = OpenRequests::default();
        q.arguments(
            [
                "app",
                "a b العربية.md",
                "https://bad/x.md",
                "--eval",
                "bad\u{fffd}.md",
                "--",
                "-literal.txt",
            ]
            .map(String::from),
            std::path::Path::new("/native"),
        );
        let (p, rejected) = q.take();
        assert!(rejected);
        assert_eq!(
            p,
            [
                PathBuf::from("/native/a b العربية.md"),
                PathBuf::from("/native/-literal.txt")
            ]
        );
        assert_eq!(q.take(), (vec![], false));
    }
    #[test]
    fn duplicate_capacity_and_expiry_do_not_create_grants() {
        let mut q = OpenRequests::default();
        q.push([PathBuf::from("same.md"), PathBuf::from("same.md")]);
        q.paths[0].1 = Instant::now() - Duration::from_secs(121);
        assert_eq!(q.take(), (vec![], true));
        q.push((0..100).map(|n| PathBuf::from(format!("{n}.md"))));
        let (p, rejected) = q.take();
        assert!(rejected);
        assert_eq!(p.len(), 16);
    }
}
