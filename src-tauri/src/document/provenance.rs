use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io::Write, sync::Arc};

const BOM: &[u8] = b"\xef\xbb\xbf";
const INDEX_STRIDE: usize = 512;
const MAX_CHANGES: usize = 1024;
const MAX_INSERT: usize = 1024 * 1024;
const MAX_PIECES: usize = 100_000;
const MAX_TOKENS: usize = 100_000;
const MAX_HISTORY_PIECES: usize = 200_000;
const MAX_SOURCE_BYTES: usize = 160 * 1024 * 1024 + 65536;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditError {
    Encoding,
    Range,
    Order,
    Insert,
    Token,
    Capacity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Insert {
    Text {
        text: String,
    },
    /// Exact deleted pieces from an earlier accepted edit in this session.
    Restore {
        token: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Change {
    pub from: usize,
    pub to: usize,
    pub insert: Insert,
}

#[derive(Clone, Copy, Debug)]
enum Eol {
    Lf,
    CrLf,
    Cr,
}
impl Eol {
    fn bytes(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
            Self::Cr => "\r",
        }
    }
    fn index(self) -> usize {
        match self {
            Self::Lf => 0,
            Self::CrLf => 1,
            Self::Cr => 2,
        }
    }
}

#[derive(Debug)]
struct Checkpoint {
    logical: usize,
    byte: usize,
    next_break: Option<(usize, Eol)>,
}
#[derive(Debug)]
struct Source {
    bytes: Arc<[u8]>,
    checkpoints: Vec<Checkpoint>, // sparse scalar boundaries, not an allocation per line
    counts: [usize; 3],
    length: usize,
}
impl Source {
    fn new(bytes: Arc<[u8]>, start: usize) -> Result<Arc<Self>, EditError> {
        let text = std::str::from_utf8(&bytes[start..]).map_err(|_| EditError::Encoding)?;
        if text.contains('\0') {
            return Err(EditError::Encoding);
        }
        let mut checkpoints = vec![Checkpoint {
            logical: 0,
            byte: start,
            next_break: None,
        }];
        let mut counts = [0; 3];
        let mut pending = 0;
        let mut byte = start;
        let mut logical = 0;
        let mut scalars = 0;
        while byte < bytes.len() {
            let (width, units, eol) = unit(&bytes, byte);
            if let Some(eol) = eol {
                counts[eol.index()] += 1;
                for point in &mut checkpoints[pending..] {
                    point.next_break = Some((logical, eol));
                }
                pending = checkpoints.len();
            }
            byte += width;
            logical += units;
            scalars += 1;
            if scalars % INDEX_STRIDE == 0 && byte < bytes.len() {
                checkpoints.push(Checkpoint {
                    logical,
                    byte,
                    next_break: None,
                });
            }
        }
        Ok(Arc::new(Self {
            bytes,
            checkpoints,
            counts,
            length: logical,
        }))
    }
    fn checkpoint(&self, position: usize) -> usize {
        self.checkpoints
            .partition_point(|point| point.logical <= position)
            - 1
    }
    fn logical_at_byte(&self, target: usize) -> usize {
        let p = &self.checkpoints[self
            .checkpoints
            .partition_point(|p| p.byte <= target)
            .saturating_sub(1)];
        let mut byte = p.byte;
        let mut logical = p.logical;
        while byte < target && byte < self.bytes.len() {
            let (width, units, _) = unit(&self.bytes, byte);
            if byte + width > target {
                break;
            }
            byte += width;
            logical += units;
        }
        logical.min(self.length)
    }
    fn byte_at(&self, position: usize) -> Result<usize, EditError> {
        if position > self.length {
            return Err(EditError::Range);
        }
        let point = &self.checkpoints[self.checkpoint(position)];
        let (mut logical, mut byte) = (point.logical, point.byte);
        while logical < position {
            let (width, units, _) = unit(&self.bytes, byte);
            byte += width;
            logical += units;
        }
        if logical != position {
            return Err(EditError::Range);
        }
        Ok(byte)
    }
    fn next_break(&self, position: usize) -> Option<(usize, Eol)> {
        let index = self.checkpoint(position);
        let next = self.checkpoints.get(index + 1);
        let end = next.map_or(self.length, |point| point.logical);
        let mut logical = position;
        let mut byte = self.byte_at(position).expect("validated boundary");
        // Scan at most one sparse-index interval, then jump past arbitrarily long lines.
        while logical < end {
            let (width, units, eol) = unit(&self.bytes, byte);
            if let Some(eol) = eol {
                return Some((logical, eol));
            }
            byte += width;
            logical += units;
        }
        next.and_then(|point| point.next_break)
    }
}

// All callers use validated UTF-8 and scalar/newline boundaries.
fn unit(bytes: &[u8], byte: usize) -> (usize, usize, Option<Eol>) {
    match bytes[byte] {
        b'\r' if bytes.get(byte + 1) == Some(&b'\n') => (2, 1, Some(Eol::CrLf)),
        b'\r' => (1, 1, Some(Eol::Cr)),
        b'\n' => (1, 1, Some(Eol::Lf)),
        _ => {
            // Validate only this scalar: scanning the whole suffix per character is quadratic.
            let width = match bytes[byte] {
                0..=0x7f => 1,
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                _ => 4,
            };
            (width, if width == 4 { 2 } else { 1 }, None)
        }
    }
}

#[derive(Clone, Debug)]
struct Piece {
    source: Arc<Source>,
    from: usize,
    to: usize,
    bytes: std::ops::Range<usize>,
}
impl Piece {
    fn slice(&self, from: usize, to: usize) -> Result<Self, EditError> {
        let start = self.from + from;
        let end = self.from + to;
        Ok(Self {
            source: self.source.clone(),
            from: start,
            to: end,
            bytes: self.source.byte_at(start)?..self.source.byte_at(end)?,
        })
    }
    fn len(&self) -> usize {
        self.to - self.from
    }
}

fn logical_of(pieces: &[Piece]) -> String {
    let mut text = String::new();
    for piece in pieces {
        let source = std::str::from_utf8(&piece.source.bytes[piece.bytes.clone()])
            .expect("validated pieces");
        let mut chars = source.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\r' {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                text.push('\n');
            } else {
                text.push(c);
            }
        }
    }
    text
}

#[derive(Debug)]
pub struct Provenance {
    bom: bool,
    pieces: Vec<Piece>,
    tokens: HashMap<String, Vec<Piece>>,
    history_pieces: usize,
    source_bytes: usize,
    length: usize,
    dominant: Eol,
}

pub(crate) struct Prepared {
    added_bytes: usize,
    pieces: Vec<Piece>,
    tokens: Vec<(String, Vec<Piece>)>,
    pub inverse: Vec<Change>,
    length: usize,
}

/// Immutable streaming input for a save barrier. Later edits cannot mutate it.
#[derive(Clone, Debug)]
pub struct Snapshot {
    bom: bool,
    pieces: Vec<Piece>,
}
impl Snapshot {
    pub fn byte_len(&self) -> usize {
        usize::from(self.bom) * 3 + self.pieces.iter().map(|p| p.bytes.len()).sum::<usize>()
    }
    pub fn read_range(
        &self,
        start: usize,
        limit: usize,
    ) -> Result<(usize, usize, String), EditError> {
        let total = self.byte_len();
        if start > total || limit > 65536 {
            return Err(EditError::Range);
        }
        let mut result = Vec::new();
        let mut offset = 0;
        let bom = if self.bom { BOM } else { &[] };
        for bytes in
            std::iter::once(bom).chain(self.pieces.iter().map(|p| &p.source.bytes[p.bytes.clone()]))
        {
            let end = offset + bytes.len();
            if end > start && result.len() < limit {
                let from = start.saturating_sub(offset);
                let n = (limit - result.len()).min(bytes.len() - from);
                result.extend_from_slice(&bytes[from..from + n]);
            }
            offset = end;
            if result.len() == limit {
                break;
            }
        }
        let mut skip = 0;
        while skip < result.len() && result[skip] & 0xc0 == 0x80 {
            skip += 1;
        }
        let mut end = result.len();
        while std::str::from_utf8(&result[skip..end]).is_err() && end > skip {
            end -= 1;
            if result.len() - end > 3 {
                return Err(EditError::Encoding);
            }
        }
        let text = std::str::from_utf8(&result[skip..end])
            .map_err(|_| EditError::Encoding)?
            .to_owned();
        Ok((start + skip, start + end, text))
    }
    pub fn write_to(&self, output: &mut impl Write) -> std::io::Result<()> {
        if self.bom {
            output.write_all(BOM)?;
        }
        for piece in &self.pieces {
            output.write_all(&piece.source.bytes[piece.bytes.clone()])?;
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProvenanceData {
    bom: bool,
    sources: Vec<(String, usize)>,
    pieces: Vec<PieceRef>,
    tokens: std::collections::BTreeMap<String, Vec<PieceRef>>,
    dominant: u8,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PieceRef {
    source: usize,
    from: usize,
    to: usize,
}

impl Snapshot {
    pub fn same_provenance(&self, other: &Self) -> bool {
        self.bom == other.bom
            && self.pieces.len() == other.pieces.len()
            && self
                .pieces
                .iter()
                .zip(&other.pieces)
                .all(|(a, b)| Arc::ptr_eq(&a.source, &b.source) && a.bytes == b.bytes)
    }
}

impl Provenance {
    pub fn new(bytes: &[u8]) -> Result<Self, EditError> {
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(EditError::Capacity);
        }
        let bom = bytes.starts_with(BOM);
        let source = Source::new(bytes.into(), if bom { BOM.len() } else { 0 })?;
        let counts = source.counts;
        // Stable tie policy: LF, CRLF, CR. No line breaks defaults to LF.
        let dominant = [Eol::Lf, Eol::CrLf, Eol::Cr]
            .into_iter()
            .max_by_key(|style| (counts[style.index()], 3 - style.index()))
            .unwrap();
        let length = source.length;
        let pieces = if length == 0 {
            vec![]
        } else {
            vec![Piece {
                bytes: source.byte_at(0)?..source.byte_at(length)?,
                source,
                from: 0,
                to: length,
            }]
        };
        Ok(Self {
            bom,
            pieces,
            tokens: HashMap::new(),
            history_pieces: 0,
            source_bytes: bytes.len(),
            length,
            dominant,
        })
    }
    pub(crate) fn checkpoint(&self) -> ProvenanceData {
        let mut ids = HashMap::<usize, usize>::new();
        let mut sources = vec![];
        let mut encode = |piece: &Piece| {
            let pointer = Arc::as_ptr(&piece.source) as usize;
            let id = *ids.entry(pointer).or_insert_with(|| {
                let id = sources.len();
                sources.push((
                    std::str::from_utf8(&piece.source.bytes)
                        .expect("UTF-8 source")
                        .to_owned(),
                    piece.source.checkpoints[0].byte,
                ));
                id
            });
            PieceRef {
                source: id,
                from: piece.from,
                to: piece.to,
            }
        };
        let pieces = self.pieces.iter().map(&mut encode).collect();
        let tokens = self
            .tokens
            .iter()
            .map(|(id, p)| (id.clone(), p.iter().map(&mut encode).collect()))
            .collect();
        ProvenanceData {
            bom: self.bom,
            sources,
            pieces,
            tokens,
            dominant: self.dominant.index() as u8,
        }
    }
    pub(crate) fn from_checkpoint(data: ProvenanceData) -> Result<Self, EditError> {
        if data.sources.iter().map(|(s, _)| s.len()).sum::<usize>() > MAX_SOURCE_BYTES
            || data.pieces.len() > MAX_PIECES
            || data.tokens.len() > MAX_TOKENS
            || data.tokens.values().map(Vec::len).sum::<usize>() > MAX_HISTORY_PIECES
        {
            return Err(EditError::Capacity);
        }
        let sources = data
            .sources
            .into_iter()
            .map(|(s, start)| {
                if !(start == 0 || (start == 3 && s.starts_with('\u{feff}'))) {
                    return Err(EditError::Encoding);
                }
                Source::new(s.into_bytes().into(), start)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let decode = |p: PieceRef| -> Result<Piece, EditError> {
            let source = sources.get(p.source).ok_or(EditError::Token)?.clone();
            if p.from >= p.to {
                return Err(EditError::Range);
            }
            Ok(Piece {
                bytes: source.byte_at(p.from)?..source.byte_at(p.to)?,
                source,
                from: p.from,
                to: p.to,
            })
        };
        let pieces = data
            .pieces
            .into_iter()
            .map(&decode)
            .collect::<Result<Vec<_>, _>>()?;
        let tokens = data
            .tokens
            .into_iter()
            .map(|(id, p)| {
                Ok((
                    id,
                    p.into_iter()
                        .map(&decode)
                        .collect::<Result<Vec<_>, EditError>>()?,
                ))
            })
            .collect::<Result<HashMap<_, _>, EditError>>()?;
        if pieces.iter().map(|p| p.bytes.len()).sum::<usize>() > MAX_SOURCE_BYTES {
            return Err(EditError::Capacity);
        }
        let source_bytes = sources.iter().map(|s| s.bytes.len()).sum();
        let length = pieces.iter().map(Piece::len).sum();
        let history_pieces = tokens.values().map(Vec::len).sum();
        let dominant = match data.dominant {
            0 => Eol::Lf,
            1 => Eol::CrLf,
            2 => Eol::Cr,
            _ => return Err(EditError::Encoding),
        };
        Ok(Self {
            bom: data.bom,
            pieces,
            tokens,
            length,
            dominant,
            history_pieces,
            source_bytes,
        })
    }
    pub fn retain_tokens(&mut self, keep: &std::collections::HashSet<String>) {
        self.tokens.retain(|id, _| keep.contains(id));
        self.history_pieces = self.tokens.values().map(Vec::len).sum();
        let mut sources = HashMap::new();
        for p in self.pieces.iter().chain(self.tokens.values().flatten()) {
            sources
                .entry(Arc::as_ptr(&p.source) as usize)
                .or_insert(p.source.bytes.len());
        }
        self.source_bytes = sources.values().sum();
    }
    pub fn backing_len(&self) -> usize {
        self.source_bytes
    }
    pub fn len_utf16(&self) -> usize {
        self.length
    }
    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
    pub fn has_bom(&self) -> bool {
        self.bom
    }
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            bom: self.bom,
            pieces: self.pieces.clone(),
        }
    }
    pub fn write_to(&self, output: &mut impl Write) -> std::io::Result<()> {
        if self.bom {
            output.write_all(BOM)?;
        }
        for piece in &self.pieces {
            output.write_all(&piece.source.bytes[piece.bytes.clone()])?;
        }
        Ok(())
    }
    /// An explicit open/checkpoint snapshot, never called for every keystroke.
    pub fn logical_text(&self) -> String {
        logical_of(&self.pieces)
    }
    pub fn utf16_at_byte(&self, target: usize) -> usize {
        let mut bytes = usize::from(self.bom) * 3;
        let mut logical = 0;
        for p in &self.pieces {
            if target < bytes + p.bytes.len() {
                let absolute = p.bytes.start + target.saturating_sub(bytes);
                return logical
                    + p.source
                        .logical_at_byte(absolute)
                        .saturating_sub(p.from)
                        .min(p.len());
            }
            bytes += p.bytes.len();
            logical += p.len();
        }
        self.length
    }
    pub fn logical_range(&self, from: usize, to: usize) -> Result<String, EditError> {
        Ok(logical_of(&self.range(from, to)?))
    }
    fn range(&self, from: usize, to: usize) -> Result<Vec<Piece>, EditError> {
        if from > to || to > self.length {
            return Err(EditError::Range);
        }
        let mut result = vec![];
        let mut cursor = 0;
        for piece in &self.pieces {
            let end = cursor + piece.len();
            // Validate zero-length endpoints too (including an insertion inside a surrogate).
            if from >= cursor && from <= end {
                piece.source.byte_at(piece.from + from - cursor)?;
            }
            if to >= cursor && to <= end {
                piece.source.byte_at(piece.from + to - cursor)?;
            }
            if from < end && to > cursor {
                result.push(
                    piece.slice(from.saturating_sub(cursor), (to - cursor).min(piece.len()))?,
                );
            }
            cursor = end;
            if cursor > to {
                break;
            }
        }
        Ok(result)
    }
    fn local_style(&self, at: usize) -> Eol {
        let mut cursor = 0;
        for piece in &self.pieces {
            let start = piece.from + at.saturating_sub(cursor).min(piece.len());
            if let Some((position, style)) = piece.source.next_break(start)
                && position < piece.to
            {
                return style;
            }
            cursor += piece.len();
        }
        self.dominant
    }
    pub(crate) fn prepare(
        &self,
        operation: &str,
        changes: &[Change],
    ) -> Result<Prepared, EditError> {
        if changes.is_empty() || changes.len() > MAX_CHANGES {
            return Err(EditError::Order);
        }
        if self.tokens.len() + changes.len() > MAX_TOKENS {
            return Err(EditError::Capacity);
        }
        let mut pieces = vec![];
        let mut undo: Vec<(usize, usize, String, Vec<Piece>)> = vec![];
        let mut cursor = 0;
        let mut output_len = 0;
        let mut last_from = None;
        let mut inserted_bytes = 0;
        let mut added_bytes = 0;
        let mut retained = self.history_pieces;
        for (index, change) in changes.iter().enumerate() {
            if change.from < cursor || last_from == Some(change.from) {
                return Err(EditError::Order);
            }
            let kept = self.range(cursor, change.from)?;
            output_len += change.from - cursor;
            pieces.extend(kept);
            let deleted = self.range(change.from, change.to)?;
            retained += deleted.len();
            if retained > MAX_HISTORY_PIECES {
                return Err(EditError::Capacity);
            }
            let token = format!("{operation}:{index}");
            if self.tokens.contains_key(&token) {
                return Err(EditError::Token);
            }
            let added = match &change.insert {
                Insert::Restore { token } => {
                    self.tokens.get(token).cloned().ok_or(EditError::Token)?
                }
                Insert::Text { text } => {
                    inserted_bytes += text.len();
                    if inserted_bytes > MAX_INSERT {
                        return Err(EditError::Capacity);
                    }
                    if text.contains(['\r', '\0']) {
                        return Err(EditError::Insert);
                    }
                    if logical_of(&deleted) == *text {
                        deleted.clone()
                    } else {
                        let encoded = text
                            .replace('\n', self.local_style(change.from).bytes())
                            .into_bytes();
                        added_bytes += encoded.len();
                        if self.source_bytes + added_bytes > MAX_SOURCE_BYTES {
                            return Err(EditError::Capacity);
                        }
                        let source = Source::new(encoded.into(), 0)?;
                        if source.length == 0 {
                            vec![]
                        } else {
                            vec![Piece {
                                bytes: 0..source.bytes.len(),
                                from: 0,
                                to: source.length,
                                source,
                            }]
                        }
                    }
                }
            };
            let count: usize = added.iter().map(Piece::len).sum();
            undo.push((output_len, output_len + count, token, deleted));
            pieces.extend(added);
            output_len += count;
            cursor = change.to;
            last_from = Some(change.from);
        }
        pieces.extend(self.range(cursor, self.length)?);
        output_len += self.length - cursor;
        // A newly adjacent lone CR + LF would collapse two logical breaks. Expand
        // only the left terminator to CRLF, retaining both logical breaks. Undo
        // includes that neighboring terminator so original bytes return exactly.
        let mut merged: Vec<Piece> = Vec::with_capacity(pieces.len());
        let mut logical = 0;
        let mut repair = 0;
        for piece in pieces {
            if let Some(last) = merged.last_mut()
                && last.source.bytes[last.bytes.end - 1] == b'\r'
                && piece.source.bytes[piece.bytes.start] == b'\n'
            {
                let original = last.slice(last.len() - 1, last.len())?;
                if !undo
                    .iter()
                    .any(|(from, to, _, _)| *from < logical && logical - 1 < *to)
                {
                    undo.push((
                        logical - 1,
                        logical,
                        format!("{operation}:repair{repair}"),
                        vec![original],
                    ));
                    repair += 1;
                }
                let prefix = last.slice(0, last.len() - 1)?;
                if prefix.len() == 0 {
                    merged.pop();
                } else {
                    *last = prefix;
                }
                added_bytes += 2;
                if self.source_bytes + added_bytes > MAX_SOURCE_BYTES {
                    return Err(EditError::Capacity);
                }
                let source = Source::new(Arc::from(&b"\r\n"[..]), 0)?;
                merged.push(Piece {
                    source,
                    from: 0,
                    to: 1,
                    bytes: 0..2,
                });
            }
            logical += piece.len();
            if let Some(last) = merged.last_mut()
                && Arc::ptr_eq(&last.source, &piece.source)
                && last.to == piece.from
            {
                last.to = piece.to;
                last.bytes.end = piece.bytes.end;
            } else {
                merged.push(piece);
            }
        }
        undo.sort_by_key(|(from, _, _, _)| *from);
        let mut groups: Vec<(usize, usize, String, Vec<Piece>)> = vec![];
        for (from, to, token, restored) in undo {
            if let Some(last) = groups.last_mut()
                && last.1 == from
            {
                last.1 = to;
                last.3.extend(restored);
            } else {
                groups.push((from, to, token, restored));
            }
        }
        let mut tokens = vec![];
        let mut inverse = vec![];
        for (from, to, token, restored) in groups {
            inverse.push(Change {
                from,
                to,
                insert: Insert::Restore {
                    token: token.clone(),
                },
            });
            tokens.push((token, restored));
        }
        // Count actual retained fragments (the preflight above also limits temporary work).
        let actual = self.history_pieces + tokens.iter().map(|(_, p)| p.len()).sum::<usize>();
        if merged.iter().map(|p| p.bytes.len()).sum::<usize>() > MAX_SOURCE_BYTES {
            return Err(EditError::Capacity);
        }
        if merged.len() > MAX_PIECES
            || actual > MAX_HISTORY_PIECES
            || self.tokens.len() + tokens.len() > MAX_TOKENS
        {
            return Err(EditError::Capacity);
        }
        Ok(Prepared {
            added_bytes,
            pieces: merged,
            tokens,
            inverse,
            length: output_len,
        })
    }
    pub(crate) fn commit(&mut self, prepared: Prepared) {
        self.source_bytes += prepared.added_bytes;
        self.pieces = prepared.pieces;
        self.length = prepared.length;
        self.history_pieces += prepared
            .tokens
            .iter()
            .map(|(_, pieces)| pieces.len())
            .sum::<usize>();
        self.tokens.extend(prepared.tokens);
    }
    pub fn apply(&mut self, operation: &str, changes: &[Change]) -> Result<Vec<Change>, EditError> {
        let prepared = self.prepare(operation, changes)?;
        let inverse = prepared.inverse.clone();
        self.commit(prepared);
        Ok(inverse)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn byte_anchor_maps_bom_crlf_unicode_and_inserted_pieces() {
        let mut source = super::Provenance::new("\u{feff}a\r\n🦀文".as_bytes()).unwrap();
        assert_eq!(source.utf16_at_byte(0), 0);
        assert_eq!(source.utf16_at_byte(3), 0);
        assert_eq!(source.utf16_at_byte(6), 2);
        assert_eq!(source.utf16_at_byte(10), 4);
        source
            .apply(
                "prefix",
                &[super::Change {
                    from: 0,
                    to: 0,
                    insert: super::Insert::Text { text: "文".into() },
                }],
            )
            .unwrap();
        assert_eq!(source.utf16_at_byte(9), 3);
    }

    use super::*;
    fn change(from: usize, to: usize, text: &str) -> Change {
        Change {
            from,
            to,
            insert: Insert::Text { text: text.into() },
        }
    }
    fn bytes(doc: &Provenance) -> Vec<u8> {
        let mut out = vec![];
        doc.write_to(&mut out).unwrap();
        out
    }
    #[test]
    fn preserves_bom_interior_bom_and_exact_mixed_eols_through_real_inverse_edits() {
        let original = "\u{feff}\u{feff}a\r\nb\nc\rd🦀".as_bytes();
        let mut doc = Provenance::new(original).unwrap();
        assert_eq!(doc.logical_text(), "\u{feff}a\nb\nc\nd🦀");
        let inverse = doc.apply("edit", &[change(2, 8, "\nX\n")]).unwrap();
        assert_eq!(bytes(&doc), "\u{feff}\u{feff}a\r\nX\r\n🦀".as_bytes());
        let redo = doc.apply("undo", &inverse).unwrap();
        assert_eq!(bytes(&doc), original);
        doc.apply("redo", &redo).unwrap();
        assert_eq!(bytes(&doc), "\u{feff}\u{feff}a\r\nX\r\n🦀".as_bytes());
    }
    #[test]
    fn chooses_surrounding_line_then_dominant_style_not_any_crlf() {
        let mut doc = Provenance::new(b"a\r\nb\nc\nlast").unwrap();
        doc.apply("one", &[change(3, 3, "\n"), change(10, 10, "\n")])
            .unwrap();
        assert_eq!(bytes(&doc), b"a\r\nb\n\nc\nlast\n");
        let mut cr = Provenance::new(b"a\rb").unwrap();
        cr.apply("cr", &[change(0, 0, "x\n")]).unwrap();
        assert_eq!(bytes(&cr), b"x\ra\rb");
    }
    #[test]
    fn validates_entire_batch_before_mutation() {
        let mut doc = Provenance::new("x🦀\r\ny".as_bytes()).unwrap();
        let original = bytes(&doc);
        for changes in [
            vec![change(0, 1, "a"), change(2, 3, "b")],
            vec![change(0, 1, "a"), change(99, 99, "b")],
            vec![change(2, 2, "x")],
            vec![change(0, 2, "x"), change(1, 3, "y")],
            vec![change(1, 1, "a"), change(1, 1, "b")],
            vec![change(0, 1, "\r")],
        ] {
            assert!(doc.apply("bad", &changes).is_err());
            assert_eq!(bytes(&doc), original);
        }
    }
    #[test]
    fn indexes_non_ascii_past_checkpoints_and_undoes_empty_document() {
        let text = format!("{}🦀end\r\n", "文".repeat(2048));
        let mut doc = Provenance::new(text.as_bytes()).unwrap();
        assert_eq!(
            doc.apply("split", &[change(2049, 2049, "x")]),
            Err(EditError::Range)
        );
        let undo = doc.apply("all", &[change(0, doc.len_utf16(), "")]).unwrap();
        assert!(doc.is_empty());
        doc.apply("undo", &undo).unwrap();
        assert_eq!(bytes(&doc), text.as_bytes());
    }
    #[test]
    fn sparse_index_storage_stays_below_a_fifth_of_backing_bytes_with_scalar_exact_seeks() {
        for text in ["a".repeat(1024 * 1024), "文🦀\r\nabc\rnext\n".repeat(65536)] {
            let source = Source::new(text.as_bytes().into(), 0).unwrap();
            let index_bytes = source.checkpoints.capacity() * std::mem::size_of::<Checkpoint>();
            assert!(
                index_bytes <= text.len() / 5 + 1024,
                "index {index_bytes}, source {}",
                text.len()
            );
            let mut seed = 19_u64;
            for _ in 0..48 {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
                let mut byte = seed as usize % text.len();
                while !text.is_char_boundary(byte) {
                    byte -= 1;
                }
                if byte > 0 && text.as_bytes()[byte - 1] == b'\r' && text.as_bytes()[byte] == b'\n'
                {
                    byte -= 1;
                }
                let logical = text[..byte]
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    .encode_utf16()
                    .count();
                assert_eq!(source.logical_at_byte(byte), logical);
                assert_eq!(source.byte_at(logical).unwrap(), byte);
            }
        }
    }
    #[test]
    fn repairs_cr_lf_join_and_restores_original_terminators_on_undo() {
        let mut doc = Provenance::new(b"a\rX\nb").unwrap();
        let undo = doc.apply("join", &[change(2, 3, "")]).unwrap();
        assert_eq!(bytes(&doc), b"a\r\n\nb");
        assert_eq!(doc.logical_text(), "a\n\nb");
        let redo = doc.apply("undo", &undo).unwrap();
        assert_eq!(bytes(&doc), b"a\rX\nb");
        doc.apply("redo", &redo).unwrap();
        assert_eq!(bytes(&doc), b"a\r\n\nb");
    }
    #[test]
    fn adjacent_deletions_and_replacements_have_one_valid_inverse() {
        for changes in [
            vec![change(0, 2, ""), change(2, 4, "")],
            vec![change(0, 2, "long"), change(2, 4, "X")],
            vec![change(0, 2, ""), change(2, 4, "Y")],
        ] {
            let mut doc = Provenance::new(b"a\r\nb\nc\r").unwrap();
            let undo = doc.apply("edit", &changes).unwrap();
            let edited = bytes(&doc);
            let redo = doc.apply("undo", &undo).unwrap();
            assert_eq!(bytes(&doc), b"a\r\nb\nc\r");
            doc.apply("redo", &redo).unwrap();
            assert_eq!(bytes(&doc), edited);
        }
    }
    #[test]
    fn replacing_unchanged_logical_text_does_not_normalize_its_newlines() {
        let original = b"a\r\nb\nc\rd";
        let mut doc = Provenance::new(original).unwrap();
        doc.apply("same", &[change(0, doc.len_utf16(), "a\nb\nc\nd")])
            .unwrap();
        assert_eq!(bytes(&doc), original);
    }
    #[test]
    fn retained_undo_fragments_are_bounded_even_when_journal_messages_are_small() {
        let mut doc = Provenance::new("ab".repeat(500).as_bytes()).unwrap();
        let changes = (0..500)
            .map(|i| change(i * 2, i * 2 + 1, "X"))
            .collect::<Vec<_>>();
        doc.apply("fragment", &changes).unwrap();
        let before = bytes(&doc);
        for index in 0..1000 {
            match doc.apply(&format!("delete{index}"), &[change(0, doc.len_utf16(), "")]) {
                Ok(inverse) => {
                    doc.apply(&format!("restore{index}"), &inverse).unwrap();
                    assert_eq!(bytes(&doc), before);
                }
                Err(EditError::Capacity) => {
                    assert_eq!(bytes(&doc), before);
                    return;
                }
                Err(error) => panic!("unexpected error: {error:?}"),
            }
        }
        panic!("unbounded undo fragment retention");
    }
    #[test]
    fn seeded_unicode_edit_sequences_match_text_oracle_and_exactly_undo() {
        let original = "\u{feff}a\r\n文\nb\r🦀\r\nlast".as_bytes();
        let mut doc = Provenance::new(original).unwrap();
        let mut expected = doc.logical_text();
        let mut history = vec![];
        let mut seed = 0x1729143u64;
        let mut random = || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            (seed >> 32) as usize
        };
        for operation in 0..1000 {
            let mut boundaries = expected
                .char_indices()
                .map(|(byte, _)| (byte, expected[..byte].encode_utf16().count()))
                .collect::<Vec<_>>();
            boundaries.push((expected.len(), expected.encode_utf16().count()));
            let a = random() % boundaries.len();
            let b = random() % boundaries.len();
            let (start, end) = (boundaries[a.min(b)], boundaries[a.max(b)]);
            let insert = ["", "🦀", "文", "\n", "x\ny", "العربية"][random() % 6];
            match doc.apply(
                &format!("edit{operation}"),
                &[change(start.1, end.1, insert)],
            ) {
                Ok(inverse) => {
                    expected.replace_range(start.0..end.0, insert);
                    assert_eq!(doc.logical_text(), expected);
                    // Independent raw-byte normalization catches accidental cross-piece CRLF fusion.
                    let raw = String::from_utf8(bytes(&doc)).unwrap();
                    assert_eq!(
                        raw.strip_prefix('\u{feff}')
                            .unwrap()
                            .replace("\r\n", "\n")
                            .replace('\r', "\n"),
                        expected
                    );
                    history.push(inverse);
                }
                Err(error) => panic!("unexpected valid edit error: {error:?}"),
            }
        }
        for (index, inverse) in history.into_iter().rev().enumerate() {
            doc.apply(&format!("undo{index}"), &inverse).unwrap();
        }
        assert_eq!(bytes(&doc), original);
    }
    #[test]
    fn restore_tokens_cannot_replicate_unbounded_output_and_checkpoints_keep_exact_undo() {
        let mut doc = Provenance::new(&vec![b'x'; 1024 * 1024]).unwrap();
        let restore = doc.apply("delete", &[change(0, 1024 * 1024, "")]).unwrap()[0]
            .insert
            .clone();
        for index in 0..(MAX_SOURCE_BYTES / (1024 * 1024)) {
            let n = doc.len_utf16();
            doc.apply(
                &format!("copy{index}"),
                &[Change {
                    from: n,
                    to: n,
                    insert: restore.clone(),
                }],
            )
            .unwrap();
        }
        let n = doc.len_utf16();
        assert_eq!(
            doc.apply(
                "overflow",
                &[Change {
                    from: n,
                    to: n,
                    insert: restore
                }]
            ),
            Err(EditError::Capacity)
        );
        assert_eq!(doc.len_utf16(), n);
        let mut mixed = Provenance::new(b"\xef\xbb\xbfa\rX\nb").unwrap();
        let undo = mixed.apply("join", &[change(2, 3, "")]).unwrap();
        let encoded = serde_json::to_vec(&mixed.checkpoint()).unwrap();
        let mut recovered =
            Provenance::from_checkpoint(serde_json::from_slice(&encoded).unwrap()).unwrap();
        recovered.apply("undo", &undo).unwrap();
        assert_eq!(bytes(&recovered), b"\xef\xbb\xbfa\rX\nb");
    }
    #[test]
    fn shared_codemirror_base_coordinate_fixture_roundtrips() {
        #[derive(Deserialize)]
        struct Fixture {
            original: String,
            expected: String,
            changes: Vec<TextChange>,
        }
        #[derive(Deserialize)]
        struct TextChange {
            from: usize,
            to: usize,
            insert: String,
        }
        let f: Fixture =
            serde_json::from_str(include_str!("../../../tests/fixtures/fidelity.json")).unwrap();
        let changes = f
            .changes
            .into_iter()
            .map(|c| change(c.from, c.to, &c.insert))
            .collect::<Vec<_>>();
        let mut doc = Provenance::new(f.original.as_bytes()).unwrap();
        let inverse = doc.apply("fixture", &changes).unwrap();
        assert_eq!(bytes(&doc), f.expected.as_bytes());
        doc.apply("undo", &inverse).unwrap();
        assert_eq!(bytes(&doc), f.original.as_bytes());
    }
}
