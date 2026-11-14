//! Opt-in saved-byte review. No document writes, path resurrection or executable bridge.
use crate::{files::FileError, private_store, scoped_file::path_key};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
};
use uuid::Uuid;
pub const CAPTURE_LIMIT: u64 = crate::files::REVIEW_CAPTURE_LIMIT;
const RECORD_LIMIT: u64 = 32 * 1024 * 1024;
fn err(message: &str) -> FileError {
    FileError::new("review", message)
}
fn hash(b: &[u8]) -> String {
    format!("{:x}", Sha256::digest(b))
}
fn uid() -> String {
    Uuid::new_v4().to_string()
}
#[derive(Clone)]
pub struct Capture {
    pub key: String,
    pub name: String,
    pub text: String,
    pub hash: String,
}
impl Capture {
    pub fn new(path: &Path, bytes: Vec<u8>) -> Result<Self, FileError> {
        if bytes.len() as u64 > CAPTURE_LIMIT {
            return Err(err(
                "Review admits saved UTF-8 files up to 2 MiB. Read and Source remain available.",
            ));
        }
        let text = String::from_utf8(bytes).map_err(|_| err("Review requires UTF-8 text."))?;
        if text.contains('\0') || line_at(&text, text.len()) > 50_000 {
            return Err(err(
                "Review refuses binary content or more than 50,000 lines.",
            ));
        }
        Ok(Self {
            key: path_key(path),
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into(),
            hash: hash(text.as_bytes()),
            text,
        })
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Anchor {
    pub revision: String,
    pub start: usize,
    pub end: usize,
    pub line_start: usize,
    pub line_end: usize,
    pub quote_hash: String,
    pub quote: String,
    pub prefix: String,
    pub suffix: String,
    pub heading_start: Option<usize>,
    pub heading_end: Option<usize>,
    pub heading_hash: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    pub id: String,
    pub text: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub anchor: Anchor,
    pub previous: Vec<Anchor>,
    pub text: String,
    pub replies: Vec<Reply>,
    pub resolved: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Data {
    schema: u32,
    version: u64,
    reviewed: Option<String>,
    viewed: Option<String>,
    decision: String,
    decision_revision: Option<String>,
    snapshots: BTreeMap<String, String>,
    comments: Vec<Comment>,
}
impl Default for Data {
    fn default() -> Self {
        Self {
            schema: 1,
            version: 0,
            reviewed: None,
            viewed: None,
            decision: "pending".into(),
            decision_revision: None,
            snapshots: BTreeMap::new(),
            comments: vec![],
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    pub id: String,
    pub key: String,
    pub name: String,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalogue {
    schema: u32,
    entries: Vec<Entry>,
}
struct Session {
    record: String,
    key: String,
    current: String,
    display_revision: String,
    frozen: Option<String>,
    shown: Option<String>,
    initial: bool,
    renamed: Option<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommentView {
    pub comment: Comment,
    pub status: String,
    pub start: Option<usize>,
    pub end: Option<usize>,
    pub line_start: Option<usize>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub record: String,
    pub token: String,
    pub version: u64,
    pub current_hash: String,
    pub text: String,
    pub reviewed_hash: Option<String>,
    pub reviewed_text: Option<String>,
    pub viewed_hash: Option<String>,
    pub viewed_text: Option<String>,
    pub initial: bool,
    pub decision: String,
    pub decision_revision: Option<String>,
    pub comments: Vec<CommentView>,
    pub snapshots: usize,
    pub renamed_to: Option<String>,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Reviewed,
    Decision {
        value: String,
    },
    Comment {
        start: usize,
        end: usize,
        quote: String,
        text: String,
        heading_start: Option<usize>,
        heading_end: Option<usize>,
    },
    Edit {
        id: String,
        text: String,
    },
    Delete {
        id: String,
    },
    Resolve {
        id: String,
        value: bool,
    },
    Reply {
        id: String,
        text: String,
    },
    EditReply {
        id: String,
        reply: String,
        text: String,
    },
    DeleteReply {
        id: String,
        reply: String,
    },
    Reanchor {
        id: String,
        start: usize,
        end: usize,
        quote: String,
        heading_start: Option<usize>,
        heading_end: Option<usize>,
    },
}
#[derive(Serialize)]
pub struct Comparison {
    pub left: String,
    pub right: String,
    pub hunks: Vec<crate::git_workbench::Hunk>,
}
pub struct Reviews {
    root: PathBuf,
    sessions: HashMap<String, Session>,
}
fn line_offsets(text: &str) -> Vec<usize> {
    let mut out = vec![0];
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\r' {
            if b.get(i + 1) == Some(&b'\n') {
                i += 1;
            }
            out.push(i + 1);
        } else if b[i] == b'\n' {
            out.push(i + 1);
        }
        i += 1;
    }
    out
}
fn line_at(text: &str, byte: usize) -> usize {
    line_offsets(text).partition_point(|p| *p <= byte.min(text.len()))
}
fn valid_anchor(a: &Anchor, text: &str, lines: &[usize]) -> bool {
    if a.start >= a.end
        || a.end - a.start > 65536
        || text.get(a.start..a.end) != Some(&a.quote)
        || hash(a.quote.as_bytes()) != a.quote_hash
    {
        return false;
    }
    if a.line_start != lines.partition_point(|p| *p <= a.start)
        || a.line_end != lines.partition_point(|p| *p < a.end)
        || a.prefix != context_before(text, a.start)
        || a.suffix != context_after(text, a.end)
    {
        return false;
    }
    match (a.heading_start, a.heading_end, &a.heading_hash) {
        (None, None, None) => true,
        (Some(start), Some(end), Some(h)) if start < end && end <= a.start => text
            .get(start..end)
            .is_some_and(|t| hash(t.as_bytes()) == *h),
        _ => false,
    }
}

fn context_before(text: &str, start: usize) -> String {
    let mut at = start.saturating_sub(96);
    while !text.is_char_boundary(at) {
        at += 1;
    }
    text[at..start].into()
}
fn context_after(text: &str, end: usize) -> String {
    let mut at = (end + 96).min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    text[end..at].into()
}
fn anchor(
    text: &str,
    revision: &str,
    start: usize,
    end: usize,
    quote: &str,
    hs: Option<usize>,
    he: Option<usize>,
) -> Result<Anchor, FileError> {
    if start >= end || end - start > 64 * 1024 || text.get(start..end) != Some(quote) {
        return Err(err(
            "Choose an exact current saved block of at most 64 KiB; this anchor is stale or invalid.",
        ));
    }
    let heading_hash = match (hs, he) {
        (Some(a), Some(b)) if a < b && b <= start => Some(hash(
            text.get(a..b)
                .ok_or_else(|| err("Invalid heading span."))?
                .as_bytes(),
        )),
        (None, None) => None,
        _ => return Err(err("Invalid heading span.")),
    };
    Ok(Anchor {
        revision: revision.into(),
        start,
        end,
        line_start: line_at(text, start),
        line_end: line_at(text, end.saturating_sub(1)),
        quote_hash: hash(quote.as_bytes()),
        quote: quote.into(),
        prefix: context_before(text, start),
        suffix: context_after(text, end),
        heading_start: hs,
        heading_end: he,
        heading_hash,
    })
}
fn reanchor(a: &Anchor, text: &str, revision: &str) -> (String, Option<usize>, Option<usize>) {
    if revision == a.revision && text.get(a.start..a.end) == Some(&a.quote) {
        return ("anchored".into(), Some(a.start), Some(a.end));
    }
    let mut matches = text.match_indices(&a.quote);
    let first = matches.next();
    if matches.next().is_some() {
        return ("ambiguous".into(), None, None);
    }
    if let Some((start, _)) = first {
        let end = start + a.quote.len();
        if text[..start].ends_with(&a.prefix) && text[end..].starts_with(&a.suffix) {
            return ("reanchored".into(), Some(start), Some(end));
        }
    }
    ("orphaned".into(), None, None)
}
fn text_ok(text: &str) -> Result<(), FileError> {
    if text.trim().is_empty() || text.len() > 4096 {
        Err(err("Use a nonempty comment or reply of at most 4 KiB."))
    } else {
        Ok(())
    }
}
impl Reviews {
    pub fn new(store: PathBuf) -> Self {
        Self {
            root: if store.as_os_str().is_empty() {
                store
            } else {
                store.join("review")
            },
            sessions: HashMap::new(),
        }
    }
    fn prepare(&self) -> Result<(), FileError> {
        if self.root.as_os_str().is_empty() {
            return Err(err("Private storage is unavailable."));
        }
        private_store::directory(&self.root)
    }
    fn catalogue(&self) -> Result<Catalogue, FileError> {
        self.prepare()?;
        let path = self.root.join("catalogue.json");
        if !path.exists() {
            return Ok(Catalogue {
                schema: 1,
                entries: vec![],
            });
        }
        let x: Catalogue = serde_json::from_slice(&private_store::read(&path, 64 * 1024)?)
            .map_err(|_| err("Unsupported or damaged review catalogue; original preserved."))?;
        if x.schema != 1
            || x.entries.len() > 128
            || x.entries
                .iter()
                .any(|e| Uuid::parse_str(&e.id).is_err() || e.key.len() != 64)
        {
            return Err(err("Unsupported review catalogue; original preserved."));
        }
        Ok(x)
    }
    fn record_path(&self, id: &str) -> Result<PathBuf, FileError> {
        Uuid::parse_str(id).map_err(|_| err("Unknown review record."))?;
        Ok(self.root.join(format!("{id}.json")))
    }
    fn load(&self, id: &str) -> Result<Data, FileError> {
        let path = self.record_path(id)?;
        let d: Data = serde_json::from_slice(&private_store::read(&path, RECORD_LIMIT)?)
            .map_err(|_| err("Unsupported or damaged review record; original preserved."))?;
        let lines: HashMap<_, _> = d
            .snapshots
            .iter()
            .map(|(h, t)| (h, line_offsets(t)))
            .collect();
        if d.schema != 1
            || !["pending", "approved", "changes_requested"].contains(&d.decision.as_str())
            || d.version >= u64::MAX - 8
            || d.comments.len() > 200
            || d.snapshots.len() > 8
            || d.snapshots
                .iter()
                .any(|(h, t)| t.len() as u64 > CAPTURE_LIMIT || hash(t.as_bytes()) != *h)
            || d.reviewed
                .iter()
                .chain(d.viewed.iter())
                .chain(d.decision_revision.iter())
                .any(|h| !d.snapshots.contains_key(h))
            || d.comments.iter().any(|c| {
                c.replies.len() > 20
                    || text_ok(&c.text).is_err()
                    || c.replies.iter().any(|r| text_ok(&r.text).is_err())
                    || c.previous.len() > 8
                    || std::iter::once(&c.anchor)
                        .chain(c.previous.iter())
                        .any(|a| {
                            !d.snapshots
                                .get(&a.revision)
                                .is_some_and(|t| valid_anchor(a, t, &lines[&a.revision]))
                        })
            })
        {
            return Err(err(
                "Review record failed schema/provenance validation; original preserved.",
            ));
        }
        Ok(d)
    }
    fn persist(&self, id: &str, d: &Data) -> Result<(), FileError> {
        let bytes = serde_json::to_vec(d).map_err(|_| err("Cannot encode review record."))?;
        if bytes.len() as u64 > RECORD_LIMIT
            || private_store::usage(self.root.parent().ok_or_else(|| err("No private store."))?)
                .map_err(|e| err(&format!("Review quota scan failed: {}", e.message)))?
                .saturating_add(bytes.len() as u64)
                > private_store::STORE_QUOTA
        {
            return Err(err(
                "Review storage budget reached. Export/delete old review data; no comments were pruned.",
            ));
        }
        private_store::atomic(&self.record_path(id)?, &bytes).map_err(|e| {
            err(&format!(
                "Review record could not be committed: {}",
                e.message
            ))
        })
    }
    fn retain(
        &self,
        id: &str,
        d: &mut Data,
        current: &str,
        frozen: Option<&String>,
    ) -> Result<(), FileError> {
        let mut pinned: HashSet<String> = HashSet::from([current.into()]);
        pinned.extend(
            d.reviewed
                .iter()
                .chain(d.viewed.iter())
                .chain(d.decision_revision.iter())
                .chain(frozen)
                .cloned(),
        );
        for c in &d.comments {
            for a in std::iter::once(&c.anchor).chain(c.previous.iter()) {
                pinned.insert(a.revision.clone());
            }
        }
        for s in self.sessions.values().filter(|s| s.record == id) {
            pinned.insert(s.current.clone());
            pinned.insert(s.display_revision.clone());
            pinned.extend(s.frozen.iter().chain(s.shown.iter()).cloned());
        }
        while d.snapshots.len() > 8 {
            let victim=d.snapshots.keys().find(|h|!pinned.contains(*h)).cloned().ok_or_else(||err("Eight review snapshots are pinned. Export/delete or explicitly reanchor comments before capturing more; records are kept."))?;
            d.snapshots.remove(&victim);
        }
        Ok(())
    }
    pub fn begin(&mut self, c: Capture) -> Result<View, FileError> {
        if self.sessions.len() >= 8 {
            return Err(err("Close another review view first."));
        }
        let mut cat = self.catalogue()?;
        let found = cat.entries.iter().find(|e| e.key == c.key).cloned();
        let entry = found.clone().unwrap_or_else(|| Entry {
            id: uid(),
            key: c.key.clone(),
            name: c.name.clone(),
        });
        if found.is_none() && cat.entries.len() >= 128 {
            return Err(err(
                "128 stored reviews reached. Export/delete an old record.",
            ));
        }
        let mut d = if self.record_path(&entry.id)?.exists() {
            self.load(&entry.id)?
        } else {
            Data::default()
        };
        let initial = d.reviewed.is_none();
        let frozen = d.viewed.clone();
        d.snapshots.insert(c.hash.clone(), c.text);
        if initial {
            d.reviewed = Some(c.hash.clone());
        }
        self.retain(&entry.id, &mut d, &c.hash, frozen.as_ref())?;
        d.version += 1;
        self.persist(&entry.id, &d)?;
        if found.is_none() {
            cat.entries.push(entry.clone());
            private_store::atomic(
                &self.root.join("catalogue.json"),
                &serde_json::to_vec(&cat).map_err(|_| err("Cannot encode review catalogue."))?,
            )?;
        }
        let token = uid();
        self.sessions.insert(
            token.clone(),
            Session {
                record: entry.id,
                key: c.key,
                display_revision: c.hash.clone(),
                current: c.hash,
                frozen,
                shown: None,
                initial,
                renamed: None,
            },
        );
        self.view(&token)
    }
    fn session(&self, token: &str) -> Result<&Session, FileError> {
        self.sessions
            .get(token)
            .ok_or_else(|| err("Review view ended. Enable Review again."))
    }
    pub fn verify(&self, token: &str, key: &str) -> Result<(), FileError> {
        if self.session(token)?.key != key {
            Err(err("Review handle belongs to another document."))
        } else {
            Ok(())
        }
    }
    pub fn view(&self, token: &str) -> Result<View, FileError> {
        let s = self.session(token)?;
        let d = self.load(&s.record)?;
        let text = d
            .snapshots
            .get(&s.current)
            .ok_or_else(|| err("Review snapshot unavailable."))?;
        let comments = d
            .comments
            .iter()
            .map(|c| {
                let (status, start, end) = reanchor(&c.anchor, text, &s.current);
                CommentView {
                    comment: c.clone(),
                    status,
                    line_start: start.map(|a| line_at(text, a)),
                    start,
                    end,
                }
            })
            .collect();
        Ok(View {
            record: s.record.clone(),
            token: token.into(),
            version: d.version,
            current_hash: s.current.clone(),
            text: text.clone(),
            reviewed_hash: d.reviewed.clone(),
            reviewed_text: d
                .reviewed
                .as_ref()
                .and_then(|h| d.snapshots.get(h))
                .cloned(),
            viewed_hash: s.frozen.clone(),
            viewed_text: s.frozen.as_ref().and_then(|h| d.snapshots.get(h)).cloned(),
            initial: s.initial,
            decision: d.decision,
            decision_revision: d.decision_revision,
            comments,
            snapshots: d.snapshots.len(),
            renamed_to: s.renamed.clone(),
        })
    }
    pub fn sync(&mut self, token: &str, c: Capture) -> Result<View, FileError> {
        self.verify(token, &c.key)?;
        let s = self.session(token)?;
        let id = s.record.clone();
        let frozen = s.frozen.clone();
        let mut d = self.load(&id)?;
        if !d.snapshots.contains_key(&c.hash) {
            d.snapshots.insert(c.hash.clone(), c.text);
            self.retain(&id, &mut d, &c.hash, frozen.as_ref())?;
            d.version += 1;
            self.persist(&id, &d)?;
        }
        self.sessions.get_mut(token).unwrap().current = c.hash;
        self.sessions.get_mut(token).unwrap().renamed = None;
        self.view(token)
    }
    pub fn sync_grant(&mut self, token: &str, c: Capture) -> Result<View, FileError> {
        let revision = c.hash.clone();
        let view = self.sync(token, c)?;
        self.sessions.get_mut(token).unwrap().display_revision = revision;
        Ok(view)
    }
    pub fn missing(&mut self, token: &str, renamed: Option<String>) -> Result<View, FileError> {
        self.sessions
            .get_mut(token)
            .ok_or_else(|| err("Review ended."))?
            .renamed = renamed;
        self.view(token)
    }
    pub fn displayed_revision(
        &mut self,
        token: &str,
        key: &str,
        revision: String,
    ) -> Result<(), FileError> {
        self.verify(token, key)?;
        let s = self.session(token)?;
        if s.display_revision != revision {
            return Err(err(
                "This revision is not the saved grant bound to this review view.",
            ));
        }
        self.sessions.get_mut(token).unwrap().shown = Some(revision);
        Ok(())
    }
    #[cfg(test)]
    fn displayed(&mut self, token: &str, c: Capture) -> Result<(), FileError> {
        self.displayed_revision(token, &c.key, c.hash)
    }
    pub fn end(&mut self, token: &str) -> Result<(), FileError> {
        let s = self.session(token)?;
        if let Some(shown) = &s.shown {
            let mut d = self.load(&s.record)?;
            d.viewed = Some(shown.clone());
            d.version += 1;
            self.persist(&s.record, &d)?;
        }
        self.sessions.remove(token);
        Ok(())
    }
    pub fn update_checked(
        &mut self,
        token: &str,
        version: u64,
        revision: &str,
        action: Action,
    ) -> Result<View, FileError> {
        if self.session(token)?.current != revision {
            return Err(err(
                "Saved capture changed since this action was prepared. Refresh before editing feedback.",
            ));
        }
        self.update(token, version, action)
    }
    pub fn update(&mut self, token: &str, version: u64, action: Action) -> Result<View, FileError> {
        let s = self.session(token)?;
        let mut d = self.load(&s.record)?;
        if d.version != version {
            return Err(err(
                "Review record changed in another view. Refresh before editing feedback.",
            ));
        }
        let text = d
            .snapshots
            .get(&s.current)
            .ok_or_else(|| err("Missing capture."))?
            .clone();
        match action {
            Action::Reviewed => d.reviewed = Some(s.current.clone()),
            Action::Decision { value } => {
                if !["pending", "approved", "changes_requested"].contains(&value.as_str()) {
                    return Err(err("Invalid review decision."));
                }
                d.decision = value;
                d.decision_revision = Some(s.current.clone());
            }
            Action::Comment {
                start,
                end,
                quote,
                text: body,
                heading_start,
                heading_end,
            } => {
                text_ok(&body)?;
                if d.comments.len() >= 200 {
                    return Err(err("200 comments reached; export/delete an old comment."));
                }
                let a = anchor(
                    &text,
                    &s.current,
                    start,
                    end,
                    &quote,
                    heading_start,
                    heading_end,
                )?;
                d.comments.push(Comment {
                    id: uid(),
                    anchor: a,
                    previous: vec![],
                    text: body,
                    replies: vec![],
                    resolved: false,
                });
            }
            other => {
                let id = match &other {
                    Action::Edit { id, .. }
                    | Action::Delete { id }
                    | Action::Resolve { id, .. }
                    | Action::Reply { id, .. }
                    | Action::EditReply { id, .. }
                    | Action::DeleteReply { id, .. }
                    | Action::Reanchor { id, .. } => id,
                    _ => unreachable!(),
                };
                let at = d
                    .comments
                    .iter()
                    .position(|c| c.id == *id)
                    .ok_or_else(|| err("Unknown comment."))?;
                let c = &mut d.comments[at];
                match other {
                    Action::Edit { text, .. } => {
                        text_ok(&text)?;
                        c.text = text;
                    }
                    Action::Delete { .. } => {
                        d.comments.remove(at);
                    }
                    Action::Resolve { value, .. } => c.resolved = value,
                    Action::Reply { text, .. } => {
                        text_ok(&text)?;
                        if c.replies.len() >= 20 {
                            return Err(err("20 replies reached."));
                        }
                        c.replies.push(Reply { id: uid(), text });
                    }
                    Action::EditReply { reply, text, .. } => {
                        text_ok(&text)?;
                        c.replies
                            .iter_mut()
                            .find(|r| r.id == reply)
                            .ok_or_else(|| err("Unknown reply."))?
                            .text = text;
                    }
                    Action::DeleteReply { reply, .. } => {
                        let n = c
                            .replies
                            .iter()
                            .position(|r| r.id == reply)
                            .ok_or_else(|| err("Unknown reply."))?;
                        c.replies.remove(n);
                    }
                    Action::Reanchor {
                        start,
                        end,
                        quote,
                        heading_start,
                        heading_end,
                        ..
                    } => {
                        if c.previous.len() >= 8 {
                            return Err(err(
                                "Eight prior anchors retained; export/delete the thread to reset its history.",
                            ));
                        }
                        let a = anchor(
                            &text,
                            &s.current,
                            start,
                            end,
                            &quote,
                            heading_start,
                            heading_end,
                        )?;
                        c.previous.push(c.anchor.clone());
                        c.anchor = a;
                    }
                    _ => unreachable!(),
                }
            }
        }
        d.version += 1;
        self.persist(&s.record, &d)?;
        self.view(token)
    }
    pub fn compare(&self, token: &str, baseline: &str) -> Result<Comparison, FileError> {
        let s = self.session(token)?;
        let d = self.load(&s.record)?;
        let h = match baseline {
            "reviewed" => d.reviewed.as_ref(),
            "viewed" => s.frozen.as_ref(),
            _ => return Err(err("Choose a saved review baseline.")),
        }
        .ok_or_else(|| err("No previous displayed saved revision exists."))?;
        let left = d
            .snapshots
            .get(h)
            .ok_or_else(|| err("Missing baseline."))?
            .clone();
        let right = d
            .snapshots
            .get(&s.current)
            .ok_or_else(|| err("Missing capture."))?
            .clone();
        let hunks = crate::git_workbench::native_review_hunks(left.as_bytes(), right.as_bytes())?;
        Ok(Comparison { left, right, hunks })
    }
    pub fn list(&self) -> Result<Vec<Entry>, FileError> {
        Ok(self.catalogue()?.entries)
    }
    pub fn delete(&mut self, id: &str) -> Result<(), FileError> {
        let mut cat = self.catalogue()?;
        if !cat.entries.iter().any(|e| e.id == id) {
            return Err(err("Unknown review record."));
        }
        let previous = serde_json::to_vec(&cat).map_err(|_| err("Cannot encode catalogue."))?;
        cat.entries.retain(|e| e.id != id);
        let catalogue = self.root.join("catalogue.json");
        // Commit the deletion intent before unlinking data. A failed catalogue
        // write must never destroy a still-listed record. A crash after this
        // commit may leave unlisted bytes, never a dangling listed record.
        private_store::atomic(
            &catalogue,
            &serde_json::to_vec(&cat).map_err(|_| err("Cannot encode catalogue."))?,
        )?;
        if let Err(error) = fs::remove_file(self.record_path(id)?) {
            return match private_store::atomic(&catalogue, &previous) {
                Ok(()) => Err(err(&format!(
                    "Review deletion failed; record retained: {error}"
                ))),
                Err(rollback) => Err(err(&format!(
                    "Review bytes retained, but catalogue rollback failed: {}. Preserve the private store for recovery.",
                    rollback.message
                ))),
            };
        }
        fs::File::open(&self.root)?.sync_all()?;
        self.sessions.retain(|_, s| s.record != id);
        Ok(())
    }
    pub fn export_view(
        &self,
        token: &str,
        path: Option<String>,
        snippets: bool,
    ) -> Result<String, FileError> {
        let s = self.session(token)?;
        self.report(&s.record, Some(&s.current), path, snippets)
    }
    pub fn export_archive(&self, id: &str, snippets: bool) -> Result<String, FileError> {
        if !self.catalogue()?.entries.iter().any(|e| e.id == id) {
            return Err(err("Unknown review record."));
        }
        self.report(id, None, None, snippets)
    }
    fn report(
        &self,
        id: &str,
        current: Option<&String>,
        path: Option<String>,
        snippets: bool,
    ) -> Result<String, FileError> {
        let d = self.load(id)?;
        let mut comments =
            serde_json::to_value(&d.comments).map_err(|_| err("Cannot encode comments."))?;
        for (c, value) in d.comments.iter().zip(comments.as_array_mut().unwrap()) {
            let (status, start, end) = if let Some(h) = current {
                reanchor(
                    &c.anchor,
                    d.snapshots.get(h).ok_or_else(|| err("Missing capture."))?,
                    h,
                )
            } else {
                ("not_evaluated".into(), None, None)
            };
            value["currentAnchor"] =
                serde_json::json!({"status":status,"revision":current,"start":start,"end":end});
            if !snippets {
                fn redact(a: &mut serde_json::Value) {
                    if let Some(o) = a.as_object_mut() {
                        o.remove("quote");
                        o.remove("prefix");
                        o.remove("suffix");
                    }
                }
                if let Some(a) = value.get_mut("anchor") {
                    redact(a);
                }
                if let Some(a) = value.get_mut("previous").and_then(|v| v.as_array_mut()) {
                    for v in a {
                        redact(v);
                    }
                }
            }
        }
        let hashes: Vec<_> = d.snapshots.keys().collect();
        serde_json::to_string_pretty(&serde_json::json!({"schema":1,"file":&path,"pathRedacted":path.is_none(),"byteCoordinates":"UTF-8 original saved bytes; end exclusive; lines one-based","pathRole":"Native-selected location; may now be missing/renamed; export restores no authority","currentAnchorRole":"Relative to the named saved capture, not unsaved editor text or a live disk assertion","currentRevision":current,"reviewedRevision":d.reviewed,"lastDisplayedRevision":d.viewed,"capturedRevisions":hashes,"decision":d.decision,"decisionRevision":d.decision_revision,"comments":comments,"snippetsIncluded":snippets,"authorAttribution":"not inferred"})).map_err(|_|err("Cannot encode feedback."))
    }
}
#[cfg(test)]
mod tests;
