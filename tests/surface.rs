//! The public surface check: a listing of the crate's public items, built
//! from the documentation rustdoc writes with every feature on, compared
//! with the checked-in listing `tests/public-items.txt`.
//!
//! The listing is built on the stable toolchain `mise.toml` pins, from
//! rustdoc's HTML: the all-items page (`all.html`) names every public item,
//! and each item's own page names what all-items does not: the fields, the
//! variants and their fields, the inherent methods and associated items, the
//! trait implementations (the auto traits `Send`, `Sync`, `Unpin`,
//! `UnwindSafe` and `RefUnwindSafe` among them, blanket implementations
//! left out), a function's or a constant's declaration, and the
//! `#[non_exhaustive]` attribute on an item or a variant. An item the crate
//! re-exports from another crate (the predicator crate's value types) is
//! listed by its path alone: its members are that crate's surface. Every
//! line is text the documentation shows, with no path of the build machine
//! in it, one line per entry, sorted.
//!
//! Two tests at this file's top level are tasks, each run on its own by a
//! `mise.toml` task after `cargo doc --no-deps --all-features` has written
//! the documentation, so the suite never runs them: `surface_check`
//! (`mise run gate:surface`), a stage of the full gate, and
//! `surface_write` (`mise run surface:write`), the one writer of the
//! listing. The other tests here are the tests of the listing's reader.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The checked-in listing, from the crate root.
const LISTING: &str = "tests/public-items.txt";

/// The crate's name as rustdoc writes it.
const CRATE: &str = "statifier";

/// The auto traits whose implementations the listing carries; the others
/// rustdoc shows are not stable to name.
const AUTO_TRAITS: [&str; 5] = ["Send", "Sync", "Unpin", "UnwindSafe", "RefUnwindSafe"];

/// The head of the checked-in listing, kept as it is by `surface_write`.
const HEAD: &str = "\
# The public items of the crate with every feature on, one per line,
# sorted: built by `mise run surface:write` from `cargo doc --no-deps
# --all-features` on the toolchain mise.toml pins, and compared with a
# listing built the same way by `mise run gate:surface`, a stage of the
# full gate. A change to the public surface changes this file in the same
# commit. Lines starting with `#` are not compared.
";

// The surface check, a stage of the full gate: the listing built from the
// documentation equals the checked-in one, and the difference is printed
// when it does not.
// Sabotage: with `pub fn statifier_probe() {}` added to src/lib.rs, and
// again with `Machine::warnings` renamed, this test fails naming the line
// added and the line removed.
#[test]
#[ignore = "a task: `mise run gate:surface` builds the documentation, then runs it"]
fn surface_check() {
    let built = build(&doc_root()).unwrap();
    let text = std::fs::read_to_string(crate_root().join(LISTING)).unwrap();
    let kept = entries(&text);
    let added: Vec<&String> = built.difference(&kept).collect();
    let removed: Vec<&String> = kept.difference(&built).collect();
    let mut report = String::new();
    for line in &removed {
        report.push_str(&format!("\n  - {line}"));
    }
    for line in &added {
        report.push_str(&format!("\n  + {line}"));
    }
    assert!(
        added.is_empty() && removed.is_empty(),
        "the public surface differs from {LISTING} (- listed only there, + built only now):{report}\nA deliberate change rewrites the listing with `mise run surface:write` in the same commit."
    );
    println!("surface: {} entries, as {LISTING} lists them", built.len());
}

// The one writer of the listing, outside the gate.
#[test]
#[ignore = "a task: `mise run surface:write` builds the documentation, then runs it"]
fn surface_write() {
    let built = build(&doc_root()).unwrap();
    let mut text = String::from(HEAD);
    for line in &built {
        text.push_str(line);
        text.push('\n');
    }
    std::fs::write(crate_root().join(LISTING), text).unwrap();
    println!("surface: wrote {} entries to {LISTING}", built.len());
}

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The crate's documentation: `doc/statifier` under the target directory,
/// `CARGO_TARGET_DIR` when it is set.
fn doc_root() -> PathBuf {
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate_root().join("target"));
    target.join("doc").join(CRATE)
}

/// The entries of a listing's text: every line that is not empty and does
/// not start with `#`.
fn entries(text: &str) -> BTreeSet<String> {
    text.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

/// The listing built from the documentation under `root`.
fn build(root: &Path) -> Result<BTreeSet<String>, String> {
    let read = |relative: &str| {
        std::fs::read_to_string(root.join(relative))
            .map_err(|error| format!("{}: {error}", root.join(relative).display()))
    };
    let all = read("all.html")?;
    let mut lines = BTreeSet::new();
    for page in item_pages(&all) {
        let Some((kind, path)) = item_of(&page) else {
            return Err(format!("all.html links {page}, which names no item"));
        };
        lines.insert(format!("{kind} {path}"));
        let mut segments: Vec<&str> = path.split("::").collect();
        segments.pop();
        for depth in 2..=segments.len() {
            lines.insert(format!("mod {}", segments[..depth].join("::")));
        }
        lines.extend(members(&path, kind, &read(&page)?));
    }
    Ok(lines)
}

/// The item pages all-items links, relative to the crate's documentation.
fn item_pages(all: &str) -> Vec<String> {
    let start = all.find("id=\"main-content\"").unwrap_or(0);
    attribute_values(&all[start..], "href=\"")
        .into_iter()
        .filter(|href| !href.starts_with("..") && !href.contains('#') && href.ends_with(".html"))
        .filter(|href| item_of(href).is_some())
        .collect()
}

/// The kind and the path of the item a page documents, read from its file
/// name: `basichttp/fn.deliver.html` is `fn statifier::basichttp::deliver`.
fn item_of(page: &str) -> Option<(&'static str, String)> {
    let (directory, file) = match page.rsplit_once('/') {
        Some((directory, file)) => (Some(directory), file),
        None => (None, page),
    };
    let (prefix, rest) = file.strip_suffix(".html")?.split_once('.')?;
    let kind = match prefix {
        "struct" => "struct",
        "enum" => "enum",
        "trait" => "trait",
        "fn" => "fn",
        "constant" => "const",
        "static" => "static",
        "type" => "type",
        "macro" => "macro",
        "union" => "union",
        "traitalias" => "trait alias",
        "derive" => "derive",
        "attr" => "attr",
        _ => return None,
    };
    let mut path = vec![CRATE];
    if let Some(directory) = directory {
        path.extend(directory.split('/'));
    }
    path.push(rest);
    Some((kind, path.join("::")))
}

/// Every value of `attribute` (written with its opening quote, as
/// `href="`) in `html`, in order.
fn attribute_values(html: &str, attribute: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find(attribute) {
        rest = &rest[start + attribute.len()..];
        let Some(end) = rest.find('"') else { break };
        values.push(rest[..end].to_owned());
        rest = &rest[end..];
    }
    values
}

/// The lines an item's page adds beside the item's own: its members, each
/// `path: ...`.
fn members(path: &str, kind: &str, html: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let line = |text: String| format!("{path}: {text}");
    let declaration = between(
        html,
        "<pre class=\"rust item-decl\"><code>",
        "</code></pre>",
    );
    let local = between(html, "<span class=\"sub-heading\">", "</span>")
        .is_some_and(|heading| heading.contains("src/"));
    if !local {
        lines.push(line(String::from("re-exported from another crate")));
        return lines;
    }
    if let Some(declaration) = declaration {
        match kind {
            "fn" | "const" | "static" | "type" => lines.push(line(one_line(&text_of(declaration)))),
            _ => lines.extend(attributes(declaration).into_iter().map(line)),
        }
    }
    let section = |name: &str| html.find(&format!("id=\"{name}\"")).unwrap_or(html.len());
    let trait_impls = section("trait-implementations-list");
    let synthetic = section("synthetic-implementations-list");
    let blanket = section("blanket-implementations-list");
    let inherent = section("implementations-list");
    let implementors = section("implementors");
    for (position, id) in ids(html) {
        let after = &html[position..];
        let entry = if id.starts_with("structfield.") {
            between(after, "<code>", "</code>")
                .map(|code| format!("field {}", one_line(&text_of(code))))
        } else if let Some(variant) = id.strip_prefix("variant.") {
            match variant.split_once(".field.") {
                Some((name, _)) => between(after, "<code>", "</code>")
                    .map(|code| format!("variant {name} field {}", one_line(&text_of(code)))),
                None if variant.contains('.') => None,
                None => header(after).map(|text| format!("variant {text}")),
            }
        } else if is_member(
            &id,
            kind,
            position,
            (inherent, trait_impls, synthetic, blanket, implementors),
        ) {
            header(after)
        } else if id.starts_with("impl-") && position > synthetic && position < blanket {
            header(after).filter(|text| {
                AUTO_TRAITS.iter().any(|name| {
                    text.starts_with(&format!("impl {name} for "))
                        || text.starts_with(&format!("impl !{name} for "))
                })
            })
        } else {
            None
        };
        if let Some(entry) = entry {
            lines.push(line(entry));
        }
    }
    lines
}

/// Whether the section `id` at `position` is one the listing carries with
/// its header: an associated item of a trait, an inherent method or
/// associated item, or a trait implementation, the auto traits apart.
/// `bounds` are where the inherent, trait, auto trait and blanket
/// implementation sections start, and where a trait's implementors do.
fn is_member(
    id: &str,
    kind: &str,
    position: usize,
    bounds: (usize, usize, usize, usize, usize),
) -> bool {
    let (inherent, trait_impls, synthetic, blanket, implementors) = bounds;
    let associated = |prefixes: &[&str]| prefixes.iter().any(|prefix| id.starts_with(prefix));
    let in_trait = kind == "trait"
        && position < implementors
        && associated(&[
            "tymethod.",
            "method.",
            "associatedtype.",
            "associatedconstant.",
        ]);
    let inherent_item = position > inherent
        && position < trait_impls.min(synthetic).min(blanket)
        && associated(&["method.", "associatedtype.", "associatedconstant."]);
    let trait_impl = id.starts_with("impl-") && position > trait_impls && position < synthetic;
    in_trait || inherent_item || trait_impl
}

/// Every `id` in `html` with the position it starts at.
fn ids(html: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut offset = 0;
    while let Some(start) = html[offset..].find(" id=\"") {
        let value = offset + start + 5;
        let Some(end) = html[value..].find('"') else {
            break;
        };
        found.push((value, html[value..value + end].to_owned()));
        offset = value + end;
    }
    found
}

/// The text of the first code header in `html`.
fn header(html: &str) -> Option<String> {
    let start = html.find("class=\"code-header\">")? + "class=\"code-header\">".len();
    let rest = &html[start..];
    let end = [rest.find("</h3>"), rest.find("</h4>")]
        .into_iter()
        .flatten()
        .min()?;
    Some(one_line(&text_of(
        &rest[..end].replace("</div>", "</div> "),
    )))
}

/// The text between the first `open` in `html` and the next `close`.
fn between<'a>(html: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = html.find(open)? + open.len();
    let end = html[start..].find(close)?;
    Some(&html[start..start + end])
}

/// The attributes a declaration shows on the item itself, as
/// `#[non_exhaustive]`: the lines before its first line of code. A
/// variant's own attribute is in the variant's header.
fn attributes(declaration: &str) -> Vec<String> {
    text_of(&declaration.replace("</div>", "</div>\n"))
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take_while(|line| line.starts_with("#["))
        .map(str::to_owned)
        .collect()
}

/// The text of an HTML fragment: its tags taken out and its entities
/// decoded, line breaks kept.
fn text_of(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for character in html.chars() {
        match character {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => text.push(character),
            _ => {}
        }
    }
    decode(&text)
}

/// `text` with the entities rustdoc writes decoded.
fn decode(text: &str) -> String {
    let mut decoded = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest.find(';').filter(|end| *end <= 10) else {
            decoded.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let character = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix('#')
                .and_then(|number| match number.strip_prefix('x') {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number.parse().ok(),
                })
                .and_then(char::from_u32),
        };
        match character {
            Some(character) => {
                decoded.push(character);
                rest = &rest[end + 1..];
            }
            None => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded.push_str(rest);
    decoded
}

/// `text` on one line: every run of white space one space, with none after
/// an opening bracket or before a closing one, and no comma before one.
fn one_line(text: &str) -> String {
    let mut line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    for (from, to) in [
        ("( ", "("),
        (" )", ")"),
        ("< ", "<"),
        (" >", ">"),
        (",)", ")"),
    ] {
        line = line.replace(from, to);
    }
    line
}

// Sabotage: with item_of answering the file name's prefix rather than the
// kind ("constant" for a constant), the first assertion fails.
#[test]
fn an_item_page_names_its_kind_and_path() {
    assert_eq!(
        item_of("basichttp/constant.URI.html"),
        Some(("const", String::from("statifier::basichttp::URI")))
    );
    assert_eq!(
        item_of("struct.Machine.html"),
        Some(("struct", String::from("statifier::Machine")))
    );
    assert_eq!(item_of("index.html"), None);
    assert_eq!(item_of("help.html"), None);
}

// Sabotage: with the `starts_with("..")` filter dropped from item_pages,
// the link to another crate's item is counted and the assert_eq fails.
#[test]
fn all_items_is_read_for_item_pages_only() {
    let all = r##"<a href="../static.files/x.css">x</a><section id="main-content">
        <a href="../statifier/index.html">statifier</a> <a href="../other/enum.Value.html">Value</a>
        <ul class="all-items"><li><a href="struct.Machine.html">Machine</a></li>
        <li><a href="basichttp/fn.deliver.html">deliver</a></li>
        <li><a href="struct.Machine.html#method.start">start</a></li></ul></section>"##;
    assert_eq!(
        item_pages(all),
        ["struct.Machine.html", "basichttp/fn.deliver.html"]
    );
}

// Sabotage: with the synthetic section's AUTO_TRAITS filter dropped, the
// Freeze implementation is listed and the assert_eq fails; with the blanket
// bound moved past the blanket section, the Any implementation is listed.
#[test]
fn a_page_lists_its_members_and_no_blanket_implementation() {
    let html = r##"<span class="sub-heading"><a class="src" href="../src/statifier/x.rs.html">Source</a></span>
<pre class="rust item-decl"><code><div class="code-attribute">#[non_exhaustive]</div>pub enum E {<details><summary class="hideme"><span>Show 2 variants</span></summary>
<div class="code-attribute">    #[non_exhaustive]</div>    A {
        x: <a href="u.html">usize</a>,
    },
    B(<a>String</a>),</details>
}</code></pre>
<section id="variant.A" class="variant"><a class="anchor">§</a><h3 class="code-header"><div class="code-attribute">#[non_exhaustive]</div>A</h3></section>
<span id="variant.A.fields"></span>
<span id="variant.A.field.x" class="section-header"><a class="anchor field">§</a><code>x: <a>usize</a></code></span>
<section id="variant.B" class="variant"><h3 class="code-header">B(<a>String</a>)</h3></section>
<h2 id="implementations">Implementations</h2><div id="implementations-list">
<section id="impl-E" class="impl"><h3 class="code-header">impl E</h3></section>
<section id="method.kind" class="method"><h4 class="code-header">pub fn <a>kind</a>(&amp;self) -&gt; &amp;<a>str</a></h4></section></div>
<h2 id="trait-implementations">Trait</h2><div id="trait-implementations-list">
<section id="impl-Clone-for-E" class="impl"><h3 class="code-header">impl <a>Clone</a> for <a>E</a></h3></section>
<section id="method.clone" class="method"><h4 class="code-header">fn <a>clone</a>(&amp;self) -&gt; E</h4></section></div>
<h2 id="synthetic-implementations">Auto</h2><div id="synthetic-implementations-list">
<section id="impl-Freeze-for-E" class="impl"><h3 class="code-header">impl Freeze for E</h3></section>
<section id="impl-Send-for-E" class="impl"><h3 class="code-header">impl <a>Send</a> for E</h3></section></div>
<h2 id="blanket-implementations">Blanket</h2><div id="blanket-implementations-list">
<section id="impl-Any-for-T" class="impl"><h3 class="code-header">impl&lt;T&gt; Any for T</h3></section></div>"##;
    assert_eq!(
        members("statifier::E", "enum", html),
        [
            "statifier::E: #[non_exhaustive]",
            "statifier::E: variant #[non_exhaustive] A",
            "statifier::E: variant A field x: usize",
            "statifier::E: variant B(String)",
            "statifier::E: pub fn kind(&self) -> &str",
            "statifier::E: impl Clone for E",
            "statifier::E: impl Send for E",
        ]
    );
}

// Sabotage: with the sub-heading's source link not required, the foreign
// page's variant is listed and the assert_eq fails.
#[test]
fn a_re_exported_item_is_listed_by_its_path_alone() {
    let html = r##"<span class="sub-heading"></span><pre class="rust item-decl"><code>pub enum Value {
    Null,
}</code></pre><section id="variant.Null" class="variant"><h3 class="code-header">Null</h3></section>"##;
    assert_eq!(
        members("statifier::Value", "enum", html),
        ["statifier::Value: re-exported from another crate"]
    );
}

// Sabotage: with one_line keeping the space after an opening bracket, the
// declaration reads `deliver( state` and the assert_eq fails.
#[test]
fn a_function_is_listed_with_its_declaration_on_one_line() {
    let html = r##"<span class="sub-heading"><a class="src" href="../../src/statifier/b.rs.html">Source</a></span><pre class="rust item-decl"><code>pub fn deliver(
    state: &amp;<a>State</a>,
    send: &amp;<a>Effect</a>,
) -&gt; <a>Result</a>&lt;<a>Delivery</a>, <a>Error</a>&gt;</code></pre>"##;
    assert_eq!(
        members("statifier::basichttp::deliver", "fn", html),
        [
            "statifier::basichttp::deliver: pub fn deliver(state: &State, send: &Effect) -> Result<Delivery, Error>"
        ]
    );
}

// Sabotage: with decode leaving `&quot;` as it is, the assert_eq fails.
#[test]
fn text_is_read_with_its_entities_decoded() {
    assert_eq!(
        text_of("<a>&lt;T&gt;</a> &amp;&quot;x&quot; &#39;y&#x27; &nbsp;& z"),
        "<T> &\"x\" 'y'  & z"
    );
}
