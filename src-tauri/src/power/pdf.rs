//! Bounded native Pandoc JSON policy and private fixed-template conversion.
use super::{Mode, PdfFonts, Prepared, Tool, err, hash, process};
use crate::{files::FileError, private_store, reader_assets};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};
const TEMPLATE: &str = r#"#set page(paper: "a4", margin: (x: 24mm, y: 22mm))
#set text(font: ("New Computer Modern", "Arial Unicode MS", "Songti SC"), size: 11pt)
#set par(justify: false, leading: 0.7em)
#set heading(numbering: none)
$if(toc)$#outline(title: [Contents])
#pagebreak()
$endif$
$body$
"#;
const LATEX: &str = r#"\documentclass[11pt]{article}
\usepackage[margin=24mm]{geometry}
\usepackage{amsmath,amssymb,graphicx,longtable,booktabs,array}
\usepackage{fontspec,adjustbox,fvextra,float}
\floatplacement{figure}{H}
\setcounter{secnumdepth}{0}
\newcounter{none}
\newcommand{\pandocbounded}[1]{\begin{adjustbox}{max width=\linewidth,max totalheight=.8\textheight}#1\end{adjustbox}}
\DefineVerbatimEnvironment{verbatim}{Verbatim}{breaklines=true,breakanywhere=true}
\IfFontExistsTF{Arial Unicode MS}{\setmainfont{Arial Unicode MS}[AutoFakeBold=2,AutoFakeSlant=.2]}{\setmainfont{lmroman10-regular.otf}[BoldFont=lmroman10-bold.otf,ItalicFont=lmroman10-italic.otf,BoldItalicFont=lmroman10-bolditalic.otf]}
\providecommand{\tightlist}{\setlength{\itemsep}{0pt}\setlength{\parskip}{0pt}}
\begin{document}
$if(toc)$\tableofcontents\newpage$endif$
$body$
\end{document}
"#;
fn typst_template(fonts: PdfFonts, dir: &Path) -> Result<String, FileError> {
    if fonts == PdfFonts::System {
        return Ok(TEMPLATE.into());
    }
    private_store::directory(&dir.join("fonts"))?;
    const FONTS: &[(&str, &[u8])] = &[
        (
            "naskh.ttf",
            include_bytes!("../../../src/fonts/assets/NotoNaskhArabic[wght].ttf"),
        ),
        (
            "cousine.ttf",
            include_bytes!("../../../src/fonts/assets/Cousine-Regular.ttf"),
        ),
        (
            "cousine-bold.ttf",
            include_bytes!("../../../src/fonts/assets/Cousine-Bold.ttf"),
        ),
    ];
    for (name, bytes) in FONTS {
        private_store::atomic(&dir.join("fonts").join(name), bytes)?;
    }
    Ok(TEMPLATE.replace(
        "\"Arial Unicode MS\", \"Songti SC\"",
        "\"Noto Naskh Arabic\"",
    ).replace("#set heading(numbering: none)", "#set heading(numbering: none)\n#show raw: set text(font: (\"Cousine\", \"Noto Naskh Arabic\"))"))
}
fn typst_args(fonts: PdfFonts) -> Vec<String> {
    let mut call = args(&[
        "compile",
        "--root",
        ".",
        "--package-path=packages",
        "--package-cache-path=packages",
    ]);
    if fonts == PdfFonts::Bundled {
        call.extend(args(&["--ignore-system-fonts", "--font-path=fonts"]));
    }
    call.extend(args(&["document.typ", "document.pdf"]));
    call
}
fn success(o: process::Output) -> Result<process::Output, FileError> {
    if o.code != Some(0) {
        return Err(err(format!(
            "Local conversion exited {:?}. Check installed tool support/packages; no destination was written.\n{}\n{}",
            o.code,
            o.stderr.chars().take(2048).collect::<String>(),
            o.stdout
                .chars()
                .rev()
                .take(4096)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<String>()
        )));
    }
    Ok(o)
}
fn args(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| (*s).into()).collect()
}
fn math_safe(source: &str) -> bool {
    // TeX translates ^^ escapes before comments/tokenization. Admit math syntax,
    // not arbitrary TeX commands, regardless of the engine's file restrictions.
    if source.len() > 16384 || source.contains("^^") {
        return false;
    }
    const COMMANDS: &str = include_str!("math-commands.txt");
    const ENVIRONMENTS: &[&str] = &[
        "matrix",
        "pmatrix",
        "bmatrix",
        "Bmatrix",
        "vmatrix",
        "Vmatrix",
        "smallmatrix",
        "cases",
        "aligned",
        "alignedat",
        "gathered",
        "array",
        "split",
        "align*",
        "alignat*",
        "equation*",
        "gather*",
        "multline*",
        "eqnarray*",
    ];
    let bytes = source.as_bytes();
    let (mut i, mut groups) = (0, 0_u32);
    let mut environments = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                while i < bytes.len() && !matches!(bytes[i], b'\n' | b'\r') {
                    i += 1;
                }
            }
            b'{' => {
                groups += 1;
                if groups > 64 {
                    return false;
                }
                i += 1;
            }
            b'}' => {
                let Some(n) = groups.checked_sub(1) else {
                    return false;
                };
                groups = n;
                i += 1;
            }
            b'\\' => {
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                if i == start {
                    if !bytes.get(i).is_some_and(|c| {
                        matches!(
                            *c,
                            b'\\'
                                | b'%'
                                | b'$'
                                | b'&'
                                | b'#'
                                | b'_'
                                | b'{'
                                | b'}'
                                | b'|'
                                | b','
                                | b':'
                                | b';'
                                | b'!'
                                | b' '
                                | b'\n'
                                | b'\r'
                                | b'\t'
                                | b'^'
                                | b'~'
                                | b'\''
                        )
                    }) {
                        return false;
                    }
                    i += 1;
                    continue;
                }
                let command = &source[start..i];
                if matches!(command, "begin" | "end") {
                    while bytes.get(i).is_some_and(|c| c.is_ascii_whitespace()) {
                        i += 1;
                    }
                    if bytes.get(i) != Some(&b'{') {
                        return false;
                    }
                    let start = i + 1;
                    let Some(end) = source[start..].find('}') else {
                        return false;
                    };
                    i = start + end + 1;
                    let environment = source[start..start + end].trim();
                    if !ENVIRONMENTS.contains(&environment) {
                        return false;
                    }
                    if command == "begin" {
                        if environments.len() >= 16 {
                            return false;
                        }
                        environments.push(environment);
                    } else if environments.pop() != Some(environment) {
                        return false;
                    }
                } else if !COMMANDS
                    .split_whitespace()
                    .any(|allowed| allowed == command)
                {
                    return false;
                }
            }
            _ => i += 1,
        }
    }
    groups == 0 && environments.is_empty()
}

fn tex_warnings(output: &process::Output) -> Vec<String> {
    let mut warnings = Vec::new();
    if !output.stderr.trim().is_empty() {
        warnings.push(output.stderr.chars().take(4096).collect());
    }
    for line in output.stdout.lines().map(str::trim) {
        if line.starts_with("Missing character:")
            || line.contains("LaTeX Font Warning:")
            || line.starts_with("LaTeX Warning:")
            || (line.starts_with("Package ") && line.contains(" Warning:"))
            || line.starts_with("Overfull \\hbox")
            || line.starts_with("Overfull \\vbox")
        {
            warnings.push(format!(
                "TeX diagnostic: {}",
                line.chars().take(512).collect::<String>()
            ));
            if warnings.len() >= 20 {
                break;
            }
        }
    }
    warnings
}

fn attr() -> Value {
    json!(["", [], []])
}
fn code(source: &str, tex: bool) -> Value {
    if tex && source.chars().count() > 32 {
        let chars = source.chars().collect::<Vec<_>>();
        let mut nodes = vec![];
        for part in chars.chunks(32) {
            if !nodes.is_empty() {
                nodes.push(json!({"t":"LineBreak"}));
            }
            nodes.push(json!({"t":"Code","c":[attr(),part.iter().collect::<String>()]}));
        }
        json!({"t":"Span","c":[attr(),nodes]})
    } else {
        json!({"t":"Code","c":[attr(),source]})
    }
}
fn valid_dimension(value: &str) -> bool {
    if value.len() > 16 {
        return false;
    }
    let (number, max) = if let Some(n) = value.strip_suffix('%') {
        (n, 100.0)
    } else {
        (value.strip_suffix("px").unwrap_or(value), 4096.0)
    };
    number
        .parse::<f64>()
        .is_ok_and(|n| n.is_finite() && n > 0.0 && n <= max)
}
pub(crate) fn math_key(source: &str, display: bool) -> String {
    format!("math:{}:{}", display, hash(source.trim().as_bytes()))
}
fn context_math(source: &str) -> bool {
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            while i < bytes.len() && !matches!(bytes[i], b'\r' | b'\n') {
                i += 1;
            }
        } else if bytes[i] == b'\\' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                i += 1;
            }
            let command = &source[start..i];
            if [
                "label",
                "ref",
                "eqref",
                "autoref",
                "nameref",
                "pageref",
                "gdef",
                "xdef",
                "global",
                "def",
                "let",
                "newcommand",
                "renewcommand",
                "providecommand",
                "DeclareMathOperator",
                "DeclarePairedDelimiter",
                "DeclarePairedDelimiterX",
                "DeclarePairedDelimiterXPP",
                "Newextarrow",
                "newenvironment",
                "renewenvironment",
                "numberwithin",
                "setcounter",
                "addtocounter",
            ]
            .contains(&command)
            {
                return true;
            }
            if command == "begin" {
                let tail = source[i..].trim_start();
                if let Some(rest) = tail.strip_prefix('{')
                    && let Some((environment, _)) = rest.split_once('}')
                    && [
                        "equation", "align", "alignat", "gather", "multline", "eqnarray",
                    ]
                    .contains(&environment.trim())
                {
                    return true;
                }
            }
            if i == start {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    false
}
fn document_context_math(value: &Value, depth: usize) -> bool {
    if depth > 64 {
        return true;
    }
    if value.get("t").and_then(Value::as_str) == Some("Math")
        && context_math(value.pointer("/c/1").and_then(Value::as_str).unwrap_or(""))
    {
        return true;
    }
    match value {
        Value::Array(values) => values.iter().any(|v| document_context_math(v, depth + 1)),
        Value::Object(values) => values.values().any(|v| document_context_math(v, depth + 1)),
        _ => false,
    }
}
fn clean(
    v: &mut Value,
    p: &Prepared,
    dir: &Path,
    warnings: &mut Vec<String>,
    nodes: &mut usize,
    depth: usize,
) -> Result<(), FileError> {
    let contextual = document_context_math(v, 0);
    clean_node(v, p, dir, warnings, nodes, depth, contextual)
}
fn clean_node(
    v: &mut Value,
    p: &Prepared,
    dir: &Path,
    warnings: &mut Vec<String>,
    nodes: &mut usize,
    depth: usize,
    contextual: bool,
) -> Result<(), FileError> {
    *nodes += 1;
    if *nodes > 100000 || depth > 64 {
        return Err(err("PDF AST exceeds its node/depth budget."));
    }
    if let Some(t) = v.get("t").and_then(Value::as_str).map(str::to_string) {
        let c = v.get("c").cloned().unwrap_or(Value::Null);
        match t.as_str() {
            "RawBlock" | "RawInline" => {
                let source = c.get(1).and_then(Value::as_str).unwrap_or("");
                *v = if t == "RawBlock" {
                    json!({"t":"CodeBlock","c":[attr(),source]})
                } else {
                    json!({"t":"Code","c":[attr(),source]})
                };
                warnings.push("Raw markup is shown as source.".into());
            }
            "Link" => {
                *v = json!({"t":"Span","c":[attr(),c.get(1).cloned().unwrap_or(json!([]))]});
            }
            "Image" => {
                let reference = c.pointer("/2/0").and_then(Value::as_str).unwrap_or("");
                if let Some(bytes) = p.capture.assets.get(reference) {
                    let image = reader_assets::image_bytes(bytes)?;
                    let extension = if image.data_url.starts_with("data:image/png") {
                        "png"
                    } else {
                        "jpg"
                    };
                    let name = format!("asset-{}.{}", hash(bytes), extension);
                    private_store::atomic(&dir.join(&name), bytes)?;
                    let mut attributes = vec![];
                    if let Some(kv) = c.pointer("/0/2").and_then(Value::as_array) {
                        for pair in kv {
                            let key = pair.get(0).and_then(Value::as_str).unwrap_or("");
                            let value = pair.get(1).and_then(Value::as_str).unwrap_or("");
                            if matches!(key, "width" | "height") && valid_dimension(value) {
                                attributes.push(pair.clone());
                            }
                        }
                    }
                    *v = json!({"t":"Image","c":[["",[],attributes],c.get(1).cloned().unwrap_or(json!([])),[name,""]]});
                } else {
                    *v = json!({"t":"Str","c":"[Image omitted: select it for export]"});
                    warnings.push("An image was omitted. Select it explicitly in Export & run before converting.".into());
                }
            }
            "CodeBlock" => {
                let lang = c.pointer("/0/1/0").and_then(Value::as_str).unwrap_or("");
                let source = c.get(1).and_then(Value::as_str).unwrap_or("");
                if lang == "mermaid" {
                    let key = format!("mermaid:{}", hash(source.trim().as_bytes()));
                    if let Some(bytes) = p.capture.assets.get(&key) {
                        reader_assets::image_bytes(bytes)?;
                        let name = format!("diagram-{}.png", hash(bytes));
                        private_store::atomic(&dir.join(&name), bytes)?;
                        *v = json!({"t":"Para","c":[{"t":"Image","c":[attr(),[{"t":"Str","c":"Diagram"}],[key,""]]}]});
                    } else {
                        warnings.push("Diagram kept as source because no validated rendered image was supplied.".into());
                        v["c"][0] = attr();
                    }
                } else {
                    v["c"][0] = attr();
                }
            }
            "Math" => {
                let source = c.get(1).and_then(Value::as_str).unwrap_or("");
                let key = math_key(
                    source,
                    c.pointer("/0/t").and_then(Value::as_str) == Some("DisplayMath"),
                );
                if let Some(bytes) = p.capture.assets.get(&key) {
                    let image = reader_assets::image_bytes(bytes)?;
                    let width = format!("{}px", image.width.div_ceil(2).max(1));
                    *v = json!({"t":"Span","c":[attr(),[{"t":"Image","c":[["",[],[["width",width]]],[{"t":"Str","c":format!("TeX math: {source}")}],[key,""]]}]]});
                } else if contextual || !math_safe(source) {
                    *v = code(source, p.request.mode == Mode::PdfTex);
                    warnings.push("Context-dependent or unsupported math was kept as source because no validated captured math raster was supplied; separately trusted local TeX is independent.".into());
                }
            }
            "Header" => v["c"][1] = attr(),
            "Code" => {
                *v = code(
                    c.get(1).and_then(Value::as_str).unwrap_or(""),
                    p.request.mode == Mode::PdfTex,
                )
            }
            "Div" | "Span" | "Table" | "Figure" => v["c"][0] = attr(),
            "Str" | "Space" | "SoftBreak" | "LineBreak" | "Para" | "Plain" | "Emph" | "Strong"
            | "Strikeout" | "Superscript" | "Subscript" | "SmallCaps" | "Quoted" | "Cite"
            | "Note" | "BlockQuote" | "OrderedList" | "BulletList" | "DefinitionList"
            | "HorizontalRule" | "LineBlock" | "AlignDefault" | "AlignLeft" | "AlignRight"
            | "AlignCenter" | "ColWidthDefault" | "ColWidth" | "DefaultStyle" | "Decimal"
            | "Example" | "LowerRoman" | "UpperRoman" | "LowerAlpha" | "UpperAlpha"
            | "DefaultDelim" | "Period" | "OneParen" | "TwoParens" | "SingleQuote"
            | "DoubleQuote" | "DisplayMath" | "InlineMath" | "AuthorInText" | "SuppressAuthor"
            | "NormalCitation" => {}
            _ => {
                return Err(err(format!(
                    "Unsupported Pandoc AST node {t}; no PDF destination was written."
                )));
            }
        }
    }
    match v {
        Value::Array(a) => {
            for child in a {
                clean_node(child, p, dir, warnings, nodes, depth + 1, contextual)?;
            }
        }
        Value::Object(o) => {
            for (k, child) in o {
                if k != "t" {
                    clean_node(child, p, dir, warnings, nodes, depth + 1, contextual)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn tex(
    tool: &Tool,
    dir: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<process::Output, FileError> {
    let mut call = args(&[
        "-no-shell-escape",
        "-interaction=nonstopmode",
        "-halt-on-error",
        "-file-line-error",
    ]);
    call.push(format!("-progname={}", tool.view.name));
    call.push(format!("-fmt={}", tool.view.name));
    call.push("document.tex".into());
    // Run twice for cross-references/TOC. No installation or network bootstrap.
    success(process::run(&tool.path, &call, dir, None, cancel, 30)?)?;
    success(process::run(&tool.path, &call, dir, None, cancel, 30)?)
}
pub fn read_pdf(path: &Path) -> Result<Vec<u8>, FileError> {
    let mut f = fs::File::open(path)
        .map_err(|_| err("The local tool produced no PDF. Check its bounded diagnostics."))?;
    if f.metadata()?.len() > 32 * 1024 * 1024 {
        return Err(err("PDF exceeds 32 MiB; no destination written."));
    }
    let mut b = vec![];
    f.by_ref().take(32 * 1024 * 1024 + 1).read_to_end(&mut b)?;
    if !b.starts_with(b"%PDF-") || b.len() > 32 * 1024 * 1024 {
        return Err(err("Tool output is not an admitted PDF."));
    }
    Ok(b)
}
pub fn convert(
    p: &Prepared,
    dir: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<(Vec<u8>, Vec<String>), FileError> {
    let pandoc = p.pandoc.as_ref().unwrap();
    let engine = p.tool.as_ref().unwrap();
    private_store::atomic(&dir.join("snapshot.txt"), p.capture.view.text.as_bytes())?;
    private_store::directory(&dir.join("data"))?;
    let format = "markdown+tex_math_dollars+tex_math_single_backslash+pipe_tables+footnotes-raw_html-raw_tex-raw_attribute-yaml_metadata_block-pandoc_title_block";
    success(process::run(
        &pandoc.path,
        &args(&[
            "--sandbox",
            "--data-dir=data",
            "--from",
            format,
            "--to=json",
            "--output=parsed.json",
            "snapshot.txt",
        ]),
        dir,
        None,
        cancel,
        20,
    )?)?;
    let mut ast: Value = serde_json::from_slice(&private_store::read(
        &dir.join("parsed.json"),
        16 * 1024 * 1024,
    )?)
    .map_err(|_| err("Pandoc produced invalid JSON."))?;
    ast["meta"] = json!({});
    let mut warnings = vec![];
    clean(&mut ast, p, dir, &mut warnings, &mut 0, 0)?;
    let digest = &p.capture.view.revision;
    let label = format!("{} · {}", p.capture.view.name, p.capture.view.origin);
    let blocks = ast["blocks"]
        .as_array_mut()
        .ok_or_else(|| err("Missing Pandoc blocks."))?;
    blocks.insert(0,json!({"t":"Para","c":[{"t":"Str","c":"SHA-256:"},{"t":"Space"},{"t":"Str","c":&digest[..32]},{"t":"Space"},{"t":"Str","c":&digest[32..]}]}));
    blocks.insert(0, json!({"t":"Para","c":[{"t":"Str","c":label}]}));
    private_store::atomic(
        &dir.join("safe.json"),
        &serde_json::to_vec(&ast).map_err(|_| err("Cannot encode safe PDF source."))?,
    )?;
    let is_tex = p.request.mode == Mode::PdfTex;
    let template = if is_tex {
        LATEX.into()
    } else {
        typst_template(p.request.pdf_fonts, dir)?
    };
    private_store::atomic(
        &dir.join(if is_tex { "layout.tex" } else { "layout.typ" }),
        template.as_bytes(),
    )?;
    let mut call = args(&[
        "--sandbox",
        "--data-dir=data",
        "--from=json",
        "--standalone",
        "--wrap=none",
        "--syntax-highlighting=none",
    ]);
    call.push(format!("--to={}", if is_tex { "latex" } else { "typst" }));
    call.push(format!(
        "--template={}",
        if is_tex { "layout.tex" } else { "layout.typ" }
    ));
    call.push(format!(
        "--output=document.{}",
        if is_tex { "tex" } else { "typ" }
    ));
    if p.request.toc {
        call.push("--toc".into());
    }
    call.push("safe.json".into());
    let output = success(process::run(&pandoc.path, &call, dir, None, cancel, 20)?)?;
    if !output.stderr.is_empty() {
        warnings.push(output.stderr.chars().take(4096).collect());
    }
    engine.validate()?;
    if is_tex {
        warnings.extend(tex_warnings(&tex(engine, dir, cancel)?));
    } else {
        let output = success(process::run(
            &engine.path,
            &typst_args(p.request.pdf_fonts),
            dir,
            None,
            cancel,
            30,
        ).map_err(|error| {
            if error.code == "timeout" && p.request.pdf_fonts == PdfFonts::System {
                FileError::new("timeout", "Typst timed out. Scanning a large installed-font collection can exceed the job limit. Try PDF fonts → Bundled fonts, or use another selected converter. No destination was written.")
            } else { error }
        })?)?;
        if !output.stderr.is_empty() {
            warnings.push(output.stderr.chars().take(4096).collect());
        }
    }
    warnings.sort();
    warnings.dedup();
    warnings.truncate(20);
    Ok((read_pdf(&dir.join("document.pdf"))?, warnings))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::*;
    #[test]
    fn ast_policy_removes_paths_raw_options_and_unsafe_math() {
        let t = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(t.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let c = Arc::new(
            Capture::new("doc".into(), b"source".to_vec(), "Saved".into(), None, None).unwrap(),
        );
        let request = Request {
            capture: c.view.id.clone(),
            block: None,
            root: None,
            database: None,
            tool: None,
            pandoc: None,
            mode: Mode::Pdf,
            toc: false,
            pdf_fonts: PdfFonts::System,
        };
        let mut p = Prepared {
            request,
            capture: c,
            block: None,
            root: None,
            database: None,
            tool: None,
            pandoc: None,
            summary: String::new(),
            created: std::time::Instant::now(),
        };
        let mut v = json!([{"t":"RawBlock","c":["typst","#read(\"/etc/passwd\")"]},{"t":"Link","c":[["x",[],[["onclick","evil"]]],[{"t":"Str","c":"local"}],["https://invalid","title"]]},{"t":"Image","c":[["",[],[]],[],["/etc/passwd",""]]},{"t":"Math","c":[{"t":"InlineMath"},"\\input{/etc/passwd}"]}]);
        let mut warnings = vec![];
        clean(&mut v, &p, t.path(), &mut warnings, &mut 0, 0).unwrap();
        assert_eq!(v[0]["t"], "CodeBlock");
        assert_eq!(v[1]["t"], "Span");
        assert_eq!(v[2]["t"], "Str");
        assert_eq!(v[3]["t"], "Code");
        assert!(!v.to_string().contains("https://"));
        assert_eq!(fs::read_dir(t.path()).unwrap().count(), 0);
        let formula = "\\abc";
        let mut captured = json!([
            {"t":"Math","c":[{"t":"DisplayMath"},"\\gdef\\abc{q}"]},
            {"t":"Math","c":[{"t":"InlineMath"},formula]}
        ]);
        let raster = include_bytes!("../../../tests/fixtures/approved-image.png").to_vec();
        Arc::make_mut(&mut p.capture)
            .assets
            .insert(math_key(formula, false), raster.clone());
        clean(&mut captured, &p, t.path(), &mut warnings, &mut 0, 0).unwrap();
        assert_eq!(captured[0]["t"], "Code");
        assert_eq!(captured[1]["t"], "Span");
        let image = &captured[1]["c"][1][0];
        assert_eq!(image["t"], "Image");
        let staged = image["c"][2][0].as_str().unwrap();
        assert!(staged.starts_with("asset-"));
        assert_eq!(fs::read(t.path().join(staged)).unwrap(), raster);
        let mut wrong_mode = json!([
            {"t":"Math","c":[{"t":"DisplayMath"},"\\gdef\\abc{q}"]},
            {"t":"Math","c":[{"t":"DisplayMath"},formula]}
        ]);
        clean(&mut wrong_mode, &p, t.path(), &mut warnings, &mut 0, 0).unwrap();
        assert_eq!(wrong_mode[1]["t"], "Code");
    }
}

#[cfg(test)]
mod local_tools {
    use super::*;
    use crate::power::*;
    use std::{sync::Arc, time::Instant};
    #[test]
    #[ignore = "Requires explicitly selected local Pandoc/Typst/TeX and an approved PNG fixture"]
    fn real_selected_tools_convert_hostile_markdown_and_trusted_tex_without_shell_escape() {
        let t = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(t.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let scratch = Scratch::new(t.path()).unwrap();
        let pandoc = Arc::new(
            Tool::select(
                Path::new(&std::env::var("MARKTUB_TEST_PANDOC").unwrap()),
                Role::Pandoc,
                &scratch.path,
            )
            .unwrap(),
        );
        let typst = Arc::new(
            Tool::select(
                Path::new(&std::env::var("MARKTUB_TEST_TYPST").unwrap()),
                Role::Typst,
                &scratch.path,
            )
            .unwrap(),
        );
        let tex = Arc::new(
            Tool::select(
                Path::new(&std::env::var("MARKTUB_TEST_TEX").unwrap()),
                Role::Tex,
                &scratch.path,
            )
            .unwrap(),
        );
        let secret = t.path().join("secret.txt");
        fs::write(&secret, "PRIVATE_CANARY_MUST_NOT_APPEAR").unwrap();
        let png = fs::read(std::env::var("MARKTUB_TEST_PNG").unwrap()).unwrap();
        let unapproved_png = t.path().join("unapproved.png");
        fs::write(&unapproved_png, &png).unwrap();
        let text = format!(
            "# PDF export check\n\nEnglish, Ελληνικά, العربية, 中文.\n\nInline $E=mc^2$ and a real display formula:\n\n$$\\int_0^1 x^2\\,dx = \\frac{{1}}{{3}}$$\n\n| Name | Value |\n|---|---|\n| **Result** | 42 |\n\nA footnote.[^note]\n\n[^note]: A real exported footnote.\n\n![Approved](selected.png){{width=120px height=60px}}\n\n![Unapproved]({})\n\n![Remote](http://127.0.0.1:9/never)\n\n```{{=typst}}\n#read(\"{}\")\n```\n\n$\\input{{{}}}$\n\n```mermaid\ngraph TD\nA-->B\n```\n",
            secret.display(),
            secret.display(),
            secret.display()
        );
        let text = format!(
            "{text}\n\nResource math: $\\includegraphics{{{}}}$.\n\nTemplate-unsupported math: $\\cancel{{x}}$. Plain root: $\\root 3\\of{{x}}$.\n\nMissing glyph: {}\n",
            unapproved_png.display(),
            '\u{10ffff}'
        );
        let mut capture = Capture::new(
            "Hostile and rich PDF".into(),
            text.into_bytes(),
            "Saved native test snapshot".into(),
            None,
            None,
        )
        .unwrap();
        // Exact valid raster fixture, generated by the test driver with CRCs.
        capture.assets.insert("selected.png".into(), png.clone());
        capture
            .assets
            .insert(format!("mermaid:{}", hash(b"graph TD\nA-->B")), png);
        let request = Request {
            capture: capture.view.id.clone(),
            block: None,
            root: None,
            database: None,
            tool: None,
            pandoc: None,
            mode: Mode::Pdf,
            toc: true,
            pdf_fonts: PdfFonts::System,
        };
        let mut prepared = Prepared {
            request,
            capture: Arc::new(capture),
            block: None,
            root: None,
            database: None,
            tool: Some(typst),
            pandoc: Some(pandoc),
            summary: String::new(),
            created: Instant::now(),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let (bytes, warnings) = convert(&prepared, &scratch.path, &cancel).unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        assert!(!warnings.is_empty());
        let artifacts = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("artifacts");
        fs::create_dir_all(&artifacts).unwrap();
        fs::write(artifacts.join("native-pdf.pdf"), bytes).unwrap();
        prepared.request.mode = Mode::PdfTex;
        prepared.tool = Some(tex.clone());
        let latex_dir = Scratch::new(t.path()).unwrap();
        let (bytes, tex_warnings) = convert(&prepared, &latex_dir.path, &cancel).unwrap();
        let latex = fs::read_to_string(latex_dir.path.join("document.tex")).unwrap();
        fs::write(artifacts.join("tex-export.tex"), &latex).unwrap();
        fs::copy(
            latex_dir.path.join("safe.json"),
            artifacts.join("tex-export-safe.json"),
        )
        .unwrap();
        assert!(!latex.contains(&format!(
            r"\includegraphics{{{}}}",
            unapproved_png.display()
        )));
        assert!(
            tex_warnings
                .iter()
                .any(|w| w.contains("Missing character:")),
            "Missing-glyph diagnostics must reach the export result: {tex_warnings:?}"
        );
        assert!(
            tex_warnings.iter().all(|w| !w.contains("Overfull")),
            "The owned source-fallback fixture must fit the PDF margins: {tex_warnings:?}"
        );
        fs::write(
            artifacts.join("tex-export-warnings.json"),
            serde_json::to_vec_pretty(&tex_warnings).unwrap(),
        )
        .unwrap();
        fs::write(artifacts.join("tex-export.pdf"), bytes).unwrap();
        let trusted_dir = Scratch::new(t.path()).unwrap();
        let escaped = t.path().join("shell-escape-marker");
        let code = format!(
            "\\documentclass{{article}}\n\\usepackage{{amsmath}}\n\\begin{{document}}Trusted local TeX: $\\frac{{1}}{{2}}$.\n\\immediate\\write18{{touch {}}}\n\\end{{document}}",
            escaped.display()
        );
        private_store::atomic(&trusted_dir.path.join("document.tex"), code.as_bytes()).unwrap();
        super::tex(&tex, &trusted_dir.path, &cancel).unwrap();
        assert!(!escaped.exists());
        let bytes = read_pdf(&trusted_dir.path.join("document.pdf")).unwrap();
        fs::write(artifacts.join("trusted-tex.pdf"), bytes).unwrap();
        println!(
            "Real PDF/TeX outputs written; source secret unchanged; shell escape marker absent."
        );
    }
}

#[cfg(test)]
mod math_context_policy {
    use super::*;
    #[test]
    fn math_catalogue_refuses_resource_encoding_and_document_escape_commands() {
        for source in [
            r"\includegraphics{unapproved.png}",
            r"\includepdf{private.pdf}",
            r"\IfFileExists{secret}{yes}{no}",
            r"\RequirePackage{shellesc}",
            r"\ExplSyntaxOn\file_input:n{secret}",
            r"\makeatletter\@@input secret",
            r"\^^69nput{secret}",
            "% hidden^^J\\input{secret}",
            r"\end{document}",
            r"\begin{document}x\end{document}",
            r"\begin{matrix}x\end{cases}",
            r"\begin{matrix}x",
            r"\special{pdf:literal}",
            r"\font\f=secret",
            r"\csname input\endcsname secret",
            r"\cancel{x}",
            r"\mathclap{x}",
            r"\bm{x}",
            "x}",
            "{x",
        ] {
            assert!(!math_safe(source), "{source}");
        }
        for source in [
            r"\frac{\alpha_i^2}{\sqrt{1+x}}",
            r"\left\{\begin{matrix}a&b\\c&d\end{matrix}\right\}",
            r"\text{literal input and graphics} + \operatorname{rank} A",
            "x % comment \\includegraphics{not executed}\n + y",
            r"\% + \{x\}",
            r"\root 3\of{x}",
        ] {
            assert!(math_safe(source), "{source}");
        }
        assert!(!math_safe(&"{".repeat(65)));
        assert!(!math_safe(&"x".repeat(16385)));
    }
    #[test]
    fn successful_tex_font_and_layout_diagnostics_are_bounded_and_visible() {
        let output = process::Output {
            code: Some(0),
            stdout: "private scratch filename\nMissing character: There is no glyph\nLaTeX Font Warning: substitute\nOverfull \\hbox (4pt too wide)\n".into(),
            stderr: String::new(),
        };
        let warnings = tex_warnings(&output);
        assert_eq!(warnings.len(), 3);
        assert!(
            warnings
                .iter()
                .all(|w| !w.contains("private scratch filename"))
        );
        let output = process::Output {
            code: Some(0),
            stdout: ("Missing character: ".to_string() + &"x".repeat(1000) + "\n").repeat(40),
            stderr: "e".repeat(8000),
        };
        let warnings = tex_warnings(&output);
        assert_eq!(warnings.len(), 20);
        assert_eq!(warnings[0].len(), 4096);
        assert!(warnings[1..].iter().all(|w| w.len() <= 528));
    }
    #[test]
    fn context_dependent_math_is_source_only_in_pdf_too() {
        for source in [
            r"x\label{a}",
            r"\eqref{later}",
            r"\gdef\x{1}",
            r"\begin{equation}x=y\end{equation}",
            r"\begin{align}x&=y\end{align}",
        ] {
            assert!(!math_safe(source), "{source}");
        }
        for source in [
            r"\tag{7}x=y",
            r"\begin{aligned}x&=y\end{aligned}",
            r"\begin{align*}x&=y\end{align*}",
        ] {
            assert!(math_safe(source), "{source}");
        }
    }
}
