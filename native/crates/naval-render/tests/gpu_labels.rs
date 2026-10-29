//! Every GPU object label written in `native/crates` is portable (`naval_render::portable_label`): ASCII,
//! without `: < > " / \ | ? *`. wgpu's DX12 backend hands a pipeline's label to DXC as the shader's file
//! name, and DXC cannot compile under a name Windows cannot open, so one bad label stops every frame on
//! Windows while macOS runs fine (twice so far: `display → Rgba8UnormSrgb`, `eye adaptation: meter`).
//!
//! The scan reads the string literals that become labels: everything inside `label: Some(…)`, the right
//! side of `let label = …;`, and, for every function or closure in the crates with a parameter named
//! `label` (naval-render's `pipelines.add` and `add_*_pipeline` helpers, each crate's own), the literal
//! passed in that position at every call (an associated function's only through its type, `Target::new(…)`, and
//! not a `self.name(…)` call in a file whose own `name` takes no label); `format!` placeholders are left out. Labels computed at run time
//! and handed to naval-render's registry are held by the assertion in `PipelineRegistry::add`.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use naval_render::{LABEL_FORBIDDEN, portable_label};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if name != "target" && name != "node_modules" {
                rust_files(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// The index just past the bracket that closes the one opened just before `from`.
fn closing(text: &[u8], from: usize, open: u8, close: u8) -> usize {
    let (mut depth, mut i, mut in_string) = (1usize, from, false);
    while i < text.len() {
        let c = text[i];
        if in_string {
            if c == b'\\' {
                i += 1;
            } else if c == b'"' {
                in_string = false;
            }
        } else if c == b'"' {
            in_string = true;
        } else if c == open {
            depth += 1;
        } else if c == close {
            depth -= 1;
            if depth == 0 {
                return i + 1;
            }
        }
        i += 1;
    }
    text.len()
}

/// The string literals in `text[start..end]`, with their byte offsets; comments are skipped.
fn literals(text: &str, start: usize, end: usize) -> Vec<(usize, String)> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = start;
    while i < end {
        if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i = text[i..end].find('\n').map_or(end, |n| i + n);
        } else if bytes[i] == b'"' {
            let from = i + 1;
            let mut j = from;
            while j < end && bytes[j] != b'"' {
                if bytes[j] == b'\\' {
                    j += 1;
                }
                j += 1;
            }
            out.push((i, unescape(&text[from..j.min(end)])));
            i = j + 1;
        } else {
            i += 1;
        }
    }
    out
}

/// A literal's value: `\n`, `\"`, `\\` and the like read as the characters they stand for.
fn unescape(literal: &str) -> String {
    let mut out = String::new();
    let mut chars = literal.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('0') => out.push('\0'),
                Some(other) => out.push(other),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The top-level arguments of the call whose parentheses span `text[start..end]` (`end` just past `)`), as
/// byte ranges; commas inside brackets, strings and comments do not split.
fn arguments(text: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let bytes = text.as_bytes();
    let (mut out, mut from, mut depth, mut i) = (Vec::new(), start, 0usize, start);
    let stop = end.saturating_sub(1);
    while i < stop {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i = text[i..stop].find('\n').map_or(stop, |n| i + n);
                continue;
            }
            b'"' => {
                i += 1;
                while i < stop && bytes[i] != b'"' {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                out.push((from, i));
                from = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if !text[from..stop].trim().is_empty() {
        out.push((from, stop));
    }
    out
}

/// A literal as its label reads, without `format!` placeholders (`{}`, `{name}`, `{:?}`).
fn without_placeholders(literal: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in literal.chars() {
        match c {
            '{' => depth += 1,
            '}' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// The type whose `impl` block holds the `fn` at `at`, if one does: `Target` for `impl Target {`,
/// `impl<T> Trait for Target<T> {` and `impl crate::gpu_labels::x::Target {`.
fn impl_type(text: &str, at: usize) -> Option<String> {
    let bytes = text.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut end = at;
    while let Some(i) = text[..end].rfind("impl") {
        end = i;
        if (i > 0 && ident(bytes[i - 1])) || !matches!(bytes.get(i + 4), Some(b' ' | b'<')) {
            continue;
        }
        let Some(open) = text[i..at].find('{').map(|o| i + o) else {
            continue;
        };
        if closing(bytes, open + 1, b'{', b'}') <= at {
            continue;
        }
        return impl_head(&text[i + 4..open]);
    }
    None
}

/// The type an `impl` head (between `impl` and `{`) is for.
fn impl_head(head: &str) -> Option<String> {
    let mut head = head.trim_start();
    if head.starts_with('<') {
        head = &head[closing(head.as_bytes(), 1, b'<', b'>')..];
    }
    let head = head.split(" where").next().unwrap_or(head);
    let ty = head.rsplit(" for ").next().unwrap_or(head);
    let ty = ty.split('<').next().unwrap_or(ty).trim();
    let ty = ty.rsplit("::").next().unwrap_or(ty);
    (!ty.is_empty() && ty.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        .then(|| ty.to_owned())
}

/// The types `text` has `impl` blocks for.
fn impl_blocks(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    text.match_indices("impl")
        .filter(|&(i, _)| {
            (i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_'))
                && matches!(bytes.get(i + 4), Some(b' ' | b'<'))
        })
        .filter_map(|(i, _)| {
            let open = i + text[i..].find(['{', ';', ')', ','])?;
            (bytes[open] == b'{')
                .then(|| impl_head(&text[i + 4..open]))
                .flatten()
        })
        .collect()
}

/// Whether a parameter list starts with a receiver (`self`, `&self`, `&'a mut self`, `mut self: Box<Self>`).
fn takes_self(params: &str) -> bool {
    let first = params.split(',').next().unwrap_or("").trim();
    let first = first.trim_start_matches('&').trim_start();
    let first = match first.strip_prefix('\'') {
        Some(rest) => rest
            .trim_start_matches(|c: char| c.is_alphanumeric() || c == '_')
            .trim_start(),
        None => first,
    };
    let first = first.strip_prefix("mut ").unwrap_or(first).trim_start();
    first == "self" || first.starts_with("self:") || first.starts_with("self ")
}

/// Label parameters by function name: which argument of a call is its label.
type Parameters = HashMap<String, BTreeSet<usize>>;

/// Functions (into `functions`, for every file) and closures (into `closures`, for this file only) with a
/// parameter named `label`, and its position among the arguments a call passes (a method's `self` does not
/// count).
fn label_parameters(text: &str, functions: &mut Parameters, closures: &mut Parameters) {
    let bytes = text.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let record = |out: &mut Parameters, name: &str, params: &str| {
        let mut index = 0;
        let mut depth = 0i32;
        let mut current = String::new();
        let mut params_list = Vec::new();
        for c in params.chars() {
            match c {
                '(' | '[' | '{' | '<' => depth += 1,
                ')' | ']' | '}' | '>' => depth -= 1,
                ',' if depth == 0 => {
                    params_list.push(std::mem::take(&mut current));
                    continue;
                }
                _ => {}
            }
            current.push(c);
        }
        params_list.push(current);
        for param in params_list {
            let param = param.trim();
            if param.is_empty()
                || param
                    .trim_start_matches(['&', ' '])
                    .trim_start_matches("mut ")
                    .starts_with("self")
            {
                continue;
            }
            if param.trim_start_matches("mut ").starts_with("label:") {
                out.entry(name.to_owned()).or_default().insert(index);
            }
            index += 1;
        }
    };
    // `fn name<…>(params)`.
    for (at, _) in text.match_indices("fn ") {
        if at > 0 && ident(bytes[at - 1]) {
            continue;
        }
        let rest = &text[at + 3..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        let Some(open) = rest.find('(') else { continue };
        let start = at + 3 + open + 1;
        let end = closing(bytes, start, b'(', b')');
        let params = &text[start..end.saturating_sub(1)];
        // An associated function without a receiver (`Target::new(device, label, size)`) is called through its
        // type, so only those calls pass it a label, not every other type's `new`.
        match impl_type(text, at).filter(|_| !takes_self(params)) {
            Some(ty) => record(functions, &format!("{ty}::{name}"), params),
            None => record(functions, &name, params),
        }
    }
    // `let [mut] name = [move] |params|`.
    for needle in ["let ", "let mut "] {
        for (at, _) in text.match_indices(needle) {
            let rest = &text[at + needle.len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            let after = rest[name.len()..].trim_start();
            let Some(after) = after.strip_prefix('=') else {
                continue;
            };
            let after = after.trim_start();
            let after = after.strip_prefix("move").unwrap_or(after).trim_start();
            let Some(params) = after.strip_prefix('|') else {
                continue;
            };
            let Some(close) = params.find('|') else {
                continue;
            };
            if !name.is_empty() {
                record(closures, &name, &params[..close]);
            }
        }
    }
}

/// The functions `text` defines (`fn name`) without a `label` parameter (`own`: this file's functions that
/// have one, from [`label_parameters`]).
fn shadowing(text: &str, own: &Parameters) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    text.match_indices("fn ")
        .filter(|&(at, _)| at == 0 || !ident(bytes[at - 1]))
        .map(|(at, _)| {
            text[at + 3..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>()
        })
        .filter(|name| !name.is_empty() && !own.contains_key(name))
        .collect()
}

/// Every `(offset, literal)` in `text` that becomes a GPU label, given every crate's functions' label
/// parameters and this file's closures'. A bare call (`name(…)`, not `.name(` or `Type::name(`) to a function
/// in `shadowed`, one this file defines without a `label` parameter, is to that function, as Rust resolves it,
/// whatever another file's function of the name takes (a test's own `contact(id, patch)` is not the theatre
/// stage's `contact(id, label, …)`, whose label is a contact's).
fn label_literals(
    text: &str,
    functions: &Parameters,
    closures: &Parameters,
    shadowed: &BTreeSet<String>,
) -> Vec<(usize, String)> {
    let bytes = text.as_bytes();
    let ident = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
    let mut found = Vec::new();
    for (at, _) in text.match_indices("label: Some(") {
        let start = at + "label: Some(".len();
        found.extend(literals(text, start, closing(bytes, start, b'(', b')')));
    }
    for needle in ["let label = ", "let mut label = "] {
        for (at, _) in text.match_indices(needle) {
            let start = at + needle.len();
            let end = text[start..].find(';').map_or(text.len(), |e| start + e);
            found.extend(literals(text, start, end));
        }
    }
    let calls = functions
        .iter()
        .map(|(name, positions)| (name, positions, shadowed.contains(name)))
        .chain(
            closures
                .iter()
                .map(|(name, positions)| (name, positions, false)),
        );
    for (name, positions, shadowed) in calls {
        // An associated function is also called as `Self::name(` inside its own type's `impl`.
        let mut needles = vec![name.clone()];
        if let Some((ty, function)) = name.split_once("::")
            && impl_blocks(text).contains(ty)
        {
            needles.push(format!("Self::{function}"));
        }
        let sites: Vec<(usize, usize)> = needles
            .iter()
            .flat_map(|needle| {
                let needle = format!("{needle}(");
                text.match_indices(&needle)
                    .map(|(at, _)| (at, needle.len()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (at, length) in sites {
            // A whole name, and a call rather than the definition.
            if (at > 0 && ident(bytes[at - 1])) || text[..at].trim_end().ends_with("fn") {
                continue;
            }
            // A bare call, or a call on `self`, to this file's own function of the name, which takes no label.
            let before = &text[..at];
            if shadowed
                && (before.ends_with("self.")
                    || (!before.ends_with('.') && !before.ends_with("::")))
            {
                continue;
            }
            let start = at + length;
            let end = closing(bytes, start, b'(', b')');
            let arguments = arguments(text, start, end);
            for &n in positions {
                if let Some(&(from, to)) = arguments.get(n) {
                    found.extend(literals(text, from, to));
                }
            }
        }
    }
    found.sort();
    found.dedup();
    found
}

#[test]
fn every_gpu_label_in_the_native_crates_is_portable() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    rust_files(&crates, &mut files);
    assert!(
        files.len() > 50,
        "found only {} sources under {}",
        files.len(),
        crates.display()
    );
    // This test names the forbidden characters itself.
    files.retain(|f| !f.ends_with("gpu_labels.rs"));
    let sources: Vec<(PathBuf, String)> = files
        .iter()
        .map(|f| (f.clone(), std::fs::read_to_string(f).unwrap()))
        .collect();
    let mut functions = Parameters::new();
    for (_, text) in &sources {
        label_parameters(text, &mut functions, &mut Parameters::new());
    }
    let mut bad = Vec::new();
    let mut checked = 0;
    for (file, text) in &sources {
        let (mut own, mut closures) = (Parameters::new(), Parameters::new());
        label_parameters(text, &mut own, &mut closures);
        let shadowed = shadowing(text, &own);
        for (at, literal) in label_literals(text, &functions, &closures, &shadowed) {
            checked += 1;
            if !portable_label(&without_placeholders(&literal)) {
                let line = text[..at].matches('\n').count() + 1;
                bad.push(format!("{}:{line}: {literal:?}", file.display()));
            }
        }
    }
    assert!(
        checked > 300,
        "only {checked} labels found: the scan no longer matches the sources"
    );
    assert!(
        bad.is_empty(),
        "GPU labels must be ASCII without {LABEL_FORBIDDEN:?} (DX12 passes them to DXC as file names):\n{}",
        bad.join("\n")
    );
}

#[test]
fn the_scan_finds_labels_in_each_form() {
    let source = r#"
        device.create_buffer(&wgpu::BufferDescriptor { label: Some("fine buffer"), size: 4 });
        let x = Desc { label: Some(if a { "left: bad" } else { "right" }), .. };
        let label = format!("shadow: {}", kind);
        let meter = pipelines.add(device, library, "eye adaptation: meter", PATH, Vec::new(), recipe);
        let p = ctx.add_scene_pipeline("ocean → surface", "shaders/ocean.wgsl", defs, recipe);
        let q = ctx.add_scene_pipeline(label, "shaders/sky.wgsl", ScenePipeline { a: (1, 2) });
        let r = pipelines.add(
            device,
            library,
            // not a label: "comment: here"
            format!("taa present {format:?}"),
            TAA_SHADER.path,
        );
        pub fn add_pipeline(&mut self, label: &str) {}
        fn add_scene_pipeline(&mut self, label: &str, shader: &str, desc: ScenePipeline) {}
        fn add(&mut self, device: &Device, library: &mut Library, label: impl Into<String>, path: &str) {}
        let read = |view: &TextureView, label: &str| make(view, label);
        let blur = read(&view, "blur: read atlas");
    "#;
    let (mut functions, mut closures) = (Parameters::new(), Parameters::new());
    label_parameters(source, &mut functions, &mut closures);
    let labels: Vec<String> = label_literals(source, &functions, &closures, &BTreeSet::new())
        .into_iter()
        .map(|(_, l)| l)
        .collect();
    for want in [
        "fine buffer",
        "left: bad",
        "right",
        "shadow: {}",
        "eye adaptation: meter",
        "ocean → surface",
        "blur: read atlas",
    ] {
        assert!(
            labels.iter().any(|l| l == want),
            "{want:?} not found in {labels:?}"
        );
    }
    assert!(
        labels.iter().any(|l| l == "taa present {format:?}"),
        "{labels:?}"
    );
    assert!(
        !labels
            .iter()
            .any(|l| l.contains("shaders/") || l.contains("comment")),
        "{labels:?}"
    );
    let bad: Vec<&String> = labels
        .iter()
        .filter(|l| !portable_label(&without_placeholders(l)))
        .collect();
    assert_eq!(bad.len(), 5, "{bad:?}");
    assert!(portable_label("sun shadows near and view maps"));
    assert!(!portable_label("display → Rgba8UnormSrgb"));
}

/// A file's own function takes its bare calls, whatever another file's of the name takes: the theatre stage's
/// `contact(id, label, …)` (a contact's label) is not a test's `contact(id, patch)`, whose patch's report once read as
/// a GPU label (`naval-client`'s `air_tests.rs`); a qualified call still goes to the other.
#[test]
fn a_files_own_function_takes_its_bare_calls() {
    let stage =
        r#"fn contact(id: &str, label: &str, reported: &str) -> TheatreContact { todo!() }"#;
    let test = r#"
        fn contact(id: &str, patch: serde_json::Value) -> TheatreContact { todo!() }
        let r2 = contact("r2", json!({"classification": "radar", "reported": "Many bogeys · 15+"}));
        let c1 = stage::contact("c1", "C-1: staged", "1 BB");
    "#;
    let mut functions = Parameters::new();
    label_parameters(stage, &mut functions, &mut Parameters::new());
    let (mut own, mut closures) = (Parameters::new(), Parameters::new());
    label_parameters(test, &mut own, &mut closures);
    let shadowed = shadowing(test, &own);
    assert!(shadowed.contains("contact"), "{shadowed:?}");
    let labels: Vec<String> = label_literals(test, &functions, &closures, &shadowed)
        .into_iter()
        .map(|(_, l)| l)
        .collect();
    assert_eq!(labels, ["C-1: staged"]);
    // In the stage's own file its bare calls are still read.
    let (mut own, mut closures) = (Parameters::new(), Parameters::new());
    let stage_call = format!("{stage}\nlet c = contact(\"c2\", \"C-2\", \"1 CA\");");
    label_parameters(&stage_call, &mut own, &mut closures);
    let labels: Vec<String> = label_literals(
        &stage_call,
        &functions,
        &closures,
        &shadowing(&stage_call, &own),
    )
    .into_iter()
    .map(|(_, l)| l)
    .collect();
    assert_eq!(labels, ["C-2"]);
}

/// An associated function's label is read only from calls through its type (the bloom's `Target::new(device, label,
/// size)` is not the theatre library's `LibraryError::new(code, message)`), and a method call on `self` goes to the
/// file's own method of the name (the sound's `self.ready(key, …)` is not the pipeline registry's `ready(label)`).
#[test]
fn associated_functions_and_own_methods_take_only_their_calls() {
    let post = r#"
        impl Target {
            fn new(device: &wgpu::Device, label: &str, size: (u32, u32)) -> Self { todo!() }
            fn twin(device: &wgpu::Device) -> Self { Self::new(device, "bloom: twin", (1, 1)) }
        }
        impl PipelineRegistry {
            fn ready(&self, label: &str) -> Option<&Pipeline> { None }
        }
        let bright = Target::new(device, "bloom: bright", size);
    "#;
    let mut functions = Parameters::new();
    label_parameters(post, &mut functions, &mut Parameters::new());
    assert!(functions.contains_key("Target::new") && functions.contains_key("ready"));
    assert!(!functions.contains_key("new"), "{functions:?}");
    let read = |text: &str| {
        let (mut own, mut closures) = (Parameters::new(), Parameters::new());
        label_parameters(text, &mut own, &mut closures);
        label_literals(text, &functions, &closures, &shadowing(text, &own))
            .into_iter()
            .map(|(_, l)| l)
            .collect::<Vec<_>>()
    };
    assert_eq!(read(post), ["bloom: twin", "bloom: bright"]);
    let elsewhere = r#"
        fn ready(&mut self, key: String, tick: u64) -> bool { true }
        let fine = self.ready(format!("{}:{name}", id), tick);
        let error = LibraryError::new("bad_params", format!("the coast {}: {}", key, e));
        let p = registry.ready("pipeline: bad");
    "#;
    assert_eq!(read(elsewhere), ["pipeline: bad"]);
}
